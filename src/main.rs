use hyper::{
    service::{make_service_fn, service_fn},
    Request, Response, Server,
};
use log::{error, info, warn};
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::io::{self, split};
use tokio_socks::tcp::Socks5Stream;

async fn handle_connect(req: Request<hyper::Body>) -> Result<Response<hyper::Body>, hyper::Error> {
    let method = req.method().clone();
    let uri = req.uri().clone();

    if method != hyper::Method::CONNECT {
        // Log non-CONNECT method request
        warn!("Received non-CONNECT request: {} {}", method, uri);
        return Ok(Response::new(hyper::Body::from("Method Not Allowed")));
    }

    // Log the CONNECT request
    info!("Handling CONNECT request for {}", uri);

    let host = req.uri().authority().unwrap().to_string();
    let proxy_addr = "127.0.0.1:12345".parse::<SocketAddr>().unwrap(); // SOCKS5 proxy address

    // Immediately return a successful response for CONNECT method
    tokio::spawn(async move {
        if let Ok(upgraded) = hyper::upgrade::on(req).await {
            match Socks5Stream::connect(proxy_addr, host).await {
                Ok(mut proxy_stream) => {
                    let (mut client_reader, mut client_writer) = split(upgraded);
                    let (mut server_reader, mut server_writer) = proxy_stream.split();

                    let client_to_server = io::copy(&mut client_reader, &mut server_writer);
                    let server_to_client = io::copy(&mut server_reader, &mut client_writer);

                    match tokio::try_join!(client_to_server, server_to_client) {
                        Ok((_, _)) => info!("Tunnel closed successfully for {}", uri),
                        Err(e) => error!("Error in tunneling for {}: {}", uri, e),
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to connect to target through SOCKS5 for {}: {}",
                        uri, e
                    );
                }
            }
        }
    });

    Ok(Response::builder()
        .status(200)
        .body(hyper::Body::empty())
        .unwrap())
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let addr = ([127, 0, 0, 1], 3129).into();

    let make_svc =
        make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle_connect)) });

    let server = Server::bind(&addr).serve(make_svc);

    info!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        error!("server error: {}", e);
    }
}
