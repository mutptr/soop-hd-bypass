use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use axum_extra::{headers, TypedHeader};
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};
use mimalloc::MiMalloc;
use regex::Regex;
use reqwest::{header, Client, StatusCode};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use tokio::time::Instant;
use tower_http::compression::CompressionLayer;
use tracing::info;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let client = ClientBuilder::new(Client::new())
        .with(Cache(HttpCache {
            mode: CacheMode::Default,
            manager: CACacheManager::default(),
            options: HttpCacheOptions::default(),
        }))
        .build();

    let app = Router::new()
        .route("/:player_link", get(handler))
        .layer(CompressionLayer::new())
        .with_state(client);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler(
    State(client): State<ClientWithMiddleware>,
    Path(player_link): Path<String>,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
) -> Result<impl IntoResponse, AppError> {
    let request = Instant::now();
    let req = client.get(format!(
        "https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/{player_link}"
    ));

    let req = match user_agent {
        Some(user_agent) => req.header(header::USER_AGENT, user_agent.as_str()),
        None => req,
    };

    let res = req.send().await?;

    info!("request {:?}", request.elapsed());

    let parse = Instant::now();

    let header_keys = [
        header::CONTENT_TYPE,
        header::AGE,
        header::CACHE_CONTROL,
        header::DATE,
        header::EXPIRES,
        header::LAST_MODIFIED,
    ];
    let headers = HeaderMap::from_iter(header_keys.into_iter().filter_map(|key| {
        res.headers()
            .get(&key)
            .map(|header_value| (key, header_value.clone()))
    }));

    let is_javascript = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|header_value| header_value.to_str().ok())
        .map(|x| x == "text/javascript")
        .unwrap_or(false);

    let status = res.status();
    let content = res.text().await?;

    let content = if status.is_success() && is_javascript {
        // a(!0),y(null),l(t),
        let regex = Regex::new(r"(.\(!0\),.\(null\)),.\(.\),.*?case 6").unwrap();
        regex.replace(&content, "$1,e.next=6;case 6").to_string()
    } else {
        content
    };

    info!("parse: {:?}", parse.elapsed());

    Ok((status, headers, content))
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
