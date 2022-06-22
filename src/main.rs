// #![feature(result_option_inspect)]
use std::pin::Pin;

use std::sync::Arc;
use std::task::Poll;
use std::{path::PathBuf, time::UNIX_EPOCH};

use faccess::PathExt;
use rocket::fs::FileServer;
use rocket::http::ContentType;
use rocket::response::Responder;
use rocket::tokio::{fs::File, sync::OnceCell};
use rocket::*;
use tokio::io::AsyncRead;
use tokio::sync::Mutex;

static TAR: OnceCell<Arc<Mutex<Option<Result<piper::Arc<Vec<u8>>, ()>>>>> = OnceCell::const_new();

struct ArcBytes(piper::Arc<Vec<u8>>, u64);
impl AsyncRead for ArcBytes {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let mut cursor = std::io::Cursor::new(&self.0[..]);
        cursor.set_position(self.1);
        let x = AsyncRead::poll_read(Pin::new(&mut cursor), cx, buf);
        self.1 = cursor.position();
        x
    }
}
impl<'r, 'o: 'r> Responder<'r, 'o> for ArcBytes {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'o> {
        Response::build()
            .header(ContentType::TAR)
            .streamed_body(self)
            .ok()
    }
}

#[get("/2b2q.tar.gz")]
async fn tar() -> Result<ArcBytes, &'static str> {
    let mut read = TAR
        .get_or_init(|| async { Default::default() })
        .await
        .lock()
        .await;
    read.as_ref()
        .map(|x| x.clone())
        .unwrap_or_else(|| {
            *read = Some(
                {
                    let mut builder = ::tar::Builder::new(vec![]);
                    builder
                        .append_dir_all(PathBuf::from("2b2q"), save_dir())
                        .and_then(|()| builder.finish())
                        .and_then(|()| builder.into_inner())
                }
                // .inspect(|x| {
                //     let mut archive = ::tar::Archive::new(std::io::Cursor::new(x));
                //     archive
                //         .entries()
                //         .map(|x| {
                //             for x in x {
                //                 x.map(|x| println!("{:?}", x.path())).ok();
                //             }
                //         })
                //         .ok();
                // })
                .map_err(drop)
                .map(piper::Arc::new),
            );
            unsafe { read.as_ref().unwrap_unchecked().clone() }
        })
        .map(|x|ArcBytes(x, 0))
        .map_err(|()| "internal error")
}

#[post("/?<token>", data = "<file>")]
async fn post(
    token: Option<&str>,
    mut file: rocket::fs::TempFile<'_>,
) -> tokio::io::Result<String> {
    use std::io::ErrorKind;
    use tokio::io::Error;
    if !tokens_contain(token).await {
        return Err(Error::new(ErrorKind::Other, "authentication"));
    }
    if let Some((path, t)) = save_path() {
        match file.copy_to(path).await {
            Ok(()) => {
                *TAR.get_or_init(|| async { Default::default() })
                    .await
                    .lock()
                    .await = None;
                Ok(t)
            }
            Err(x) => Err(x),
        }
    } else {
        Err(Error::new(ErrorKind::Other, "internal error"))
    }
}

fn save_dir() -> PathBuf {
    std::env::var("SAVE_DIR")
        .unwrap_or_else(|_| ".".into())
        .into()
}
fn save_file_ext() -> Option<String> {
    std::env::var("SAVE_FILE_EXT").ok()
}
fn save_path() -> Option<(PathBuf, String)> {
    UNIX_EPOCH.elapsed().ok().map(|t| {
        let mut path = save_dir();
        let t = t.as_nanos().to_string();
        path.push(&t);
        if let Some(ext) = save_file_ext() {
            path.set_extension(ext);
        }
        (path, t)
    })
}
fn token_file() -> PathBuf {
    std::env::var("TOKEN_FILE")
        .unwrap_or_else(|_| "tokens.txt".into())
        .into()
}

async fn tokens_contain(t: Option<&str>) -> bool {
    use rocket::tokio::io::AsyncBufReadExt;
    static TOKENS: OnceCell<Option<Vec<String>>> = OnceCell::const_new();
    TOKENS
        .get_or_init(|| async {
            let mut file = match File::open(token_file()).await {
                Ok(f) => rocket::tokio::io::BufReader::new(f),
                Err(_) => return None,
            }
            .lines();
            let mut tokens = vec![];
            while let Ok(Some(x)) = file.next_line().await {
                if x != "" {
                    tokens.push(x);
                }
            }
            (tokens.len() != 0).then_some(tokens)
        })
        .await
        .as_ref()
        .map(|tokens| tokens.iter().any(|x| Some(&x[..]) == t))
        .unwrap_or(true)
}

#[launch]
fn rocket() -> _ {
    dotenv::dotenv().ok();
    assert!(save_dir().exists(), "SAVE_DIR doesn't exist!");
    assert!(save_dir().writable(), "SAVE_DIR readonly");
    rocket::build().mount("/", routes![post, tar]).mount(
        "/",
        FileServer::new(save_dir(), rocket::fs::Options::NormalizeDirs),
    )
}
