//! Web UI auth (D§13, plan S4.C / T3.2): the scoped UI token opens `/ui`
//! and nothing else; every mutating API route refuses it.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower_server::auth::ui_token;
use tower_server::serve;

const TOKEN: &str = "t"; // common::boot's bearer token

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

async fn send(app: &axum::Router, req: Request<Body>) -> Resp {
    let resp = tower::ServiceExt::oneshot(app.clone(), req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    Resp {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn req(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder().method(method).uri(uri)
}

async fn tcp() -> (common::Ctx, axum::Router) {
    let ctx = common::boot().await;
    let app = serve::with_tcp_auth(ctx.router.clone(), TOKEN);
    (ctx, app)
}

#[tokio::test]
async fn ui_token_opens_the_ui_and_nothing_else() {
    let (_ctx, app) = tcp().await;
    let ui = ui_token(TOKEN);
    let cookie = format!("theme=dark; tower_ui={ui}");

    let page = send(
        &app,
        req("GET", "/ui")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("agent cloud"));
    let js = send(
        &app,
        req("GET", "/ui/assets/topcoat-runtime-0.9.0.js")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(js.status, StatusCode::OK);
    // the runtime's page re-render: POST + header, rendered as a GET
    let rerun = send(
        &app,
        req("POST", "/ui")
            .header(header::COOKIE, &cookie)
            .header("x-topcoat-runtime", "true")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"signals":{}}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(rerun.status, StatusCode::OK);
    // any other POST under /ui is refused
    let post = send(
        &app,
        req("POST", "/ui")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(post.status, StatusCode::UNAUTHORIZED);

    // the API is closed to it, reads included, as cookie or as bearer
    for (method, uri) in [
        ("GET", "/v1/agents"),
        ("GET", "/v1/events"),
        ("GET", "/v1/ui/token"),
    ] {
        let r = send(
            &app,
            req(method, uri)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(
            r.status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} via cookie"
        );
        let r = send(
            &app,
            req(method, uri)
                .header(header::AUTHORIZATION, format!("Bearer {ui}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(
            r.status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} via bearer"
        );
    }
}

#[tokio::test]
async fn every_mutating_route_refuses_the_ui_token() {
    let (_ctx, app) = tcp().await;
    let ui = ui_token(TOKEN);
    let schema = send(
        &app,
        req("GET", "/v1/schema")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&schema.body).unwrap();
    let mut checked = 0;
    for r in v["routes"].as_array().unwrap() {
        let method = r["method"].as_str().unwrap();
        if method == "GET" {
            continue;
        }
        let path = r["path"].as_str().unwrap().replace("{id}", "x");
        for (name, value) in [
            (header::COOKIE, format!("tower_ui={ui}")),
            (header::AUTHORIZATION, format!("Bearer {ui}")),
        ] {
            let resp = send(
                &app,
                req(method, &path)
                    .header(name.clone(), &value)
                    .header("x-topcoat-runtime", "true")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await;
            assert_eq!(
                resp.status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} via {name}"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 20,
        "registry lists the mutating routes ({checked})"
    );
}

#[tokio::test]
async fn login_link_sets_a_scoped_cookie_and_leaves_the_address_bar() {
    let (_ctx, app) = tcp().await;
    let ui = ui_token(TOKEN);

    let bad = send(
        &app,
        req("GET", "/ui/login?token=nope")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    assert!(bad.headers.get(header::SET_COOKIE).is_none());
    assert!(
        bad.body.contains("tower ui"),
        "tells the operator what to run"
    );
    // the bearer token is not a UI login
    let bearer = send(
        &app,
        req("GET", &format!("/ui/login?token={TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(bearer.status, StatusCode::UNAUTHORIZED);

    let ok = send(
        &app,
        req("GET", &format!("/ui/login?token={ui}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER);
    assert_eq!(ok.headers[header::LOCATION], "/ui");
    let set = ok.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set.starts_with(&format!("tower_ui={ui};")));
    for attr in ["HttpOnly", "SameSite=Strict", "Path=/ui"] {
        assert!(set.contains(attr), "{attr} in {set}");
    }

    let none = send(&app, req("GET", "/ui").body(Body::empty()).unwrap()).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    assert!(none.body.contains("tower ui"));
    let root = send(&app, req("GET", "/").body(Body::empty()).unwrap()).await;
    assert_eq!(root.headers[header::LOCATION], "/ui");
}

#[tokio::test]
async fn bearer_fetches_the_ui_token_which_follows_the_bearer() {
    let (_ctx, app) = tcp().await;
    let r = send(
        &app,
        req("GET", "/v1/ui/token")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(v["token"], ui_token(TOKEN));
    assert_eq!(v["login_path"], "/ui/login");
    assert_ne!(
        ui_token(TOKEN),
        ui_token("rotated"),
        "rotating the bearer rotates it"
    );
    assert_ne!(ui_token(TOKEN), TOKEN);
}
