use activitypub::collection::OrderedCollection;
use atom_syndication::{Entry, FeedBuilder};
use rocket::{
    request::LenientForm,
    response::{Redirect, Flash, content::Content},
    http::ContentType
};
use rocket_contrib::Template;
use serde_json;
use std::{collections::HashMap, borrow::Cow};
use validator::{Validate, ValidationError, ValidationErrors};

use plume_common::activity_pub::{ActivityStream, ApRequest};
use plume_common::utils;
use plume_models::{
    blog_authors::*,
    blogs::*,
    db_conn::DbConn,
    instance::Instance,
    posts::Post,
    users::User
};
use routes::Page;

#[get("/~/<name>?<page>", rank = 2)]
fn paginated_details(name: String, conn: DbConn, user: Option<User>, page: Page) -> Template {
    may_fail!(user.map(|u| u.to_json(&*conn)), Blog::find_by_fqn(&*conn, name), "Requested blog couldn't be found", |blog| {
        let posts = Post::blog_page(&*conn, &blog, page.limits());
        let articles = Post::get_for_blog(&*conn, &blog);
        let authors = &blog.list_authors(&*conn);

        Template::render("blogs/details", json!({
            "blog": &blog.to_json(&*conn),
            "account": user.clone().map(|u| u.to_json(&*conn)),
            "is_author": user.map(|x| x.is_author_in(&*conn, blog.clone())),
            "posts": posts.into_iter().map(|p| p.to_json(&*conn)).collect::<Vec<serde_json::Value>>(),
            "authors": authors.into_iter().map(|u| u.to_json(&*conn)).collect::<Vec<serde_json::Value>>(),
            "n_authors": authors.len(),
            "n_articles": articles.len(),
            "page": page.page,
            "n_pages": Page::total(articles.len() as i32)
        }))
    })
}

#[get("/~/<name>", rank = 3)]
fn details(name: String, conn: DbConn, user: Option<User>) -> Template {
    paginated_details(name, conn, user, Page::first())
}

#[get("/~/<name>", rank = 1)]
fn activity_details(name: String, conn: DbConn, _ap: ApRequest) -> ActivityStream<CustomGroup> {
    let blog = Blog::find_local(&*conn, name).unwrap();
    ActivityStream::new(blog.into_activity(&*conn))
}

#[get("/blogs/new")]
fn new(user: User, conn: DbConn) -> Template {
    Template::render("blogs/new", json!({
        "account": user.to_json(&*conn),
        "errors": null,
        "form": null
    }))
}

#[get("/blogs/new", rank = 2)]
fn new_auth() -> Flash<Redirect>{
    utils::requires_login(
        "You need to be logged in order to create a new blog",
        uri!(new).into()
    )
}

#[derive(FromForm, Validate, Serialize)]
struct NewBlogForm {
    #[validate(custom(function = "valid_slug", message = "Invalid name"))]
    pub title: String
}

fn valid_slug(title: &str) -> Result<(), ValidationError> {
    let slug = utils::make_actor_id(title.to_string());
    if slug.len() == 0 {
        Err(ValidationError::new("empty_slug"))
    } else {
        Ok(())
    }
}

#[post("/blogs/new", data = "<data>")]
fn create(conn: DbConn, data: LenientForm<NewBlogForm>, user: User) -> Result<Redirect, Template> {
    let form = data.get();
    let slug = utils::make_actor_id(form.title.to_string());

    let mut errors = match form.validate() {
        Ok(_) => ValidationErrors::new(),
        Err(e) => e
    };
    if let Some(_) = Blog::find_local(&*conn, slug.clone()) {
        errors.add("title", ValidationError {
            code: Cow::from("existing_slug"),
            message: Some(Cow::from("A blog with the same name already exists.")),
            params: HashMap::new()
        });
    }

    if errors.is_empty() {
        let blog = Blog::insert(&*conn, NewBlog::new_local(
            slug.clone(),
            form.title.to_string(),
            String::from(""),
            Instance::local_id(&*conn)
        ));
        blog.update_boxes(&*conn);

        BlogAuthor::insert(&*conn, NewBlogAuthor {
            blog_id: blog.id,
            author_id: user.id,
            is_owner: true
        });

        Ok(Redirect::to(uri!(details: name = slug.clone())))
    } else {
        println!("{:?}", errors);
        Err(Template::render("blogs/new", json!({
            "account": user.to_json(&*conn),
            "errors": errors.inner(),
            "form": form
        })))
    }
}

#[get("/~/<name>/outbox")]
fn outbox(name: String, conn: DbConn) -> ActivityStream<OrderedCollection> {
    let blog = Blog::find_local(&*conn, name).unwrap();
    blog.outbox(&*conn)
}

#[get("/~/<name>/atom.xml")]
fn atom_feed(name: String, conn: DbConn) -> Content<String> {
    let blog = Blog::find_by_fqn(&*conn, name.clone()).expect("Unable to find blog");
    let feed = FeedBuilder::default()
        .title(blog.title.clone())
        .id(Instance::get_local(&*conn).unwrap().compute_box("~", name, "atom.xml"))
        .entries(Post::get_recents_for_blog(&*conn, &blog, 15)
            .into_iter()
            .map(|p| super::post_to_atom(p, &*conn))
            .collect::<Vec<Entry>>())
        .build()
        .expect("Error building Atom feed");
    Content(ContentType::new("application", "atom+xml"), feed.to_string())
}
