use std::path::{Path, PathBuf};
use log::info;
use rocket::fs::NamedFile;
use rocket::request::{FromRequest, Outcome};
const STATIC_SUFFIXES: [&str; 8] = [
    &"js", &"css", &"mp3", &"html", &"jpg", &"ttf", &"otf", &"gif",
];
use rocket::{get, Request};

// Copied from botlisk - not sure the best way to handle reusing this
pub (in super) struct StaticAsset {}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for StaticAsset {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let path = request.uri().path();
        let filename = match path.segments().last() {
            Some(f) => f,
            None => return Outcome::Failure((rocket::http::Status::NotFound, ())),
        };
        let suffix = match filename.rsplit('.').next() {
            None => {
                return Outcome::Failure((rocket::http::Status::NotFound, ()));
            }
            Some(s) => s,
        };
        if STATIC_SUFFIXES.contains(&suffix) {
            Outcome::Success(StaticAsset {})
        } else {
            Outcome::Failure((rocket::http::Status::NotFound, ()))
        }
    }
}

#[get("/<file..>")]
pub(in super) async fn statics(file: PathBuf, _asset: StaticAsset) -> Option<NamedFile> {
    let p = Path::new("http/static/").join(file);
    if !p.exists() {
        info!("{:?} does not exist", p);
        return None;
    }
    NamedFile::open(p).await.ok()
}

#[get("/favicon.ico")]
pub (in super) async fn favicon() -> Option<NamedFile> {
    let p = Path::new("http/static/favicon.ico");
    NamedFile::open(p).await.ok()
}
