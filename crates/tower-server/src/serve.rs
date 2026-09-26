//! `tower serve`: boot the server (plan T3.1-T3.4, D§3-4).

use anyhow::Context;

use crate::api;
use crate::auth;
use crate::config::Config;
use crate::paths::{load_or_create_token, Paths};
use crate::sse;
use crate::state::AppState;
use crate::storage::{open as open_db, EventLog};

pub async fn serve() -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    paths.ensure_dirs()?;
    let config = Config::load(&paths)?;
    let token = load_or_create_token(&paths)?;

    let pool = open_db(&paths.db_file).await?;
    let events = EventLog::attach(&pool).await?;

    let state = AppState::new(pool, events, config.clone(), token.clone());

    let ev = state
        .events
        .append(
            tower_core::EventKind::ServerStarted,
            None,
            None,
            serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() }),
        )
        .await?;
    tracing::info!(seq = ev.seq, "server.started");

    let routes = api::router()
        .merge(axum::Router::new().route("/v1/events", axum::routing::get(sse::events)))
        .with_state(state);

    // TCP: bearer-token auth (D§13).
    let tcp_addr: std::net::SocketAddr = config
        .server
        .bind_tcp
        .parse()
        .context("invalid server.bind_tcp address")?;
    let tcp_app = routes.clone().layer(axum::middleware::from_fn_with_state(
        auth::Auth {
            token: token.clone(),
        },
        auth::tcp_auth,
    ));

    let tcp_listener = tokio::net::TcpListener::bind(tcp_addr).await?;
    tracing::info!(%tcp_addr, "listening (tcp, token required)");

    let mut tasks = vec![];
    if config.server.bind_socket {
        let socket_path = paths.socket_file.clone();
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }
        let unix_listener = tokio::net::UnixListener::bind(&socket_path)?;
        tracing::info!(?socket_path, "listening (unix socket)");

        // Unix socket: auth exemption via ViaSocket marker middleware.
        let socket_app = routes.layer(axum::middleware::from_fn(
            |mut req: axum::extract::Request, next: axum::middleware::Next| async move {
                req.extensions_mut().insert(auth::ViaSocket);
                next.run(req).await
            },
        ));

        tasks.push(tokio::spawn(async move {
            if let Err(e) = serve_unix(unix_listener, socket_app).await {
                tracing::error!(error = %e, "unix listener failed");
            }
        }));
    }

    println!(
        "tower server {} — one port, one database",
        env!("CARGO_PKG_VERSION")
    );
    println!("  socket  {}", paths.socket_file.display());
    println!("  tcp     {} (token required)", tcp_addr);
    println!("  db      {}", paths.db_file.display());
    println!("  token   {} (0600)", paths.token_file.display());

    tasks.push(tokio::spawn(async move {
        if let Err(e) = axum::serve(tcp_listener, tcp_app).await {
            tracing::error!(error = %e, "tcp listener failed");
        }
    }));

    for t in tasks {
        t.await?;
    }
    Ok(())
}

/// Serve HTTP over a unix socket. `axum::Router` implements
/// `tower::Service<Request<Body>>`; hyper-util drives it directly.
async fn serve_unix(listener: tokio::net::UnixListener, app: axum::Router) -> anyhow::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        let app = app.clone();
        let hyper_service =
            hyper::service::service_fn(move |req: http::Request<hyper::body::Incoming>| {
                let mut app = app.clone();
                async move {
                    let resp = tower_service::Service::call(&mut app, req).await?;
                    Ok::<_, std::convert::Infallible>(resp)
                }
            });
        tokio::spawn(async move {
            let stream = stream;
            let io = hyper_util::rt::TokioIo::new(Box::pin(stream));
            let builder =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
            let conn = builder.serve_connection(io, hyper_service);
            conn.await
        });
    }
}
