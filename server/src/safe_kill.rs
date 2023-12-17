use tokio::signal;

use std;

use hyper::body::Incoming;

use axum::http::Request;

use tower::Service;

use hyper_util::rt::TokioIo;

use tokio::sync::watch;

use tokio;

use tokio::sync::watch::Sender;

use axum::Router;

pub(crate) async fn serve(server: Router) -> Sender<()> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:48228")
        .await
        .unwrap();

    let (close_tx, close_rx) = watch::channel(());

    loop {
        let (socket, _) = tokio::select! {
            result = listener.accept() => {
                result.unwrap()
            }
            _ = shutdown_signal() => {
                println!("cancelled connection");
                break;
            }
        };

        let tower = server.clone();
        let close_rx = close_rx.clone();

        tokio::spawn(async move {
            let socket = TokioIo::new(socket);
            let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
                tower.clone().call(request)
            });

            let conn = hyper::server::conn::http1::Builder::new()
                .serve_connection(socket, hyper_service)
                .with_upgrades(); // future

            let mut conn = std::pin::pin!(conn);

            loop {
                tokio::select! {
                    result = conn.as_mut() => {
                        if result.is_err() {
                            println!("req failed");
                        }
                        break;
                    }
                    _ = shutdown_signal() => {
                        println!("starting shutdown");
                        conn.as_mut().graceful_shutdown();
                    }
                }
            }

            drop(close_rx);
        });
    }

    drop(listener);

    close_tx
}

pub(crate) async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.unwrap();
    };

    ctrl_c.await
}
