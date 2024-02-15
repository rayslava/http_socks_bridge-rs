use hyper::{
    service::{make_service_fn, service_fn},
    Body, Method, Request, Response, Server, StatusCode,
};
use log::{error, info, warn};
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio_socks::tcp::Socks5Stream;

async fn proxy_http_through_socks5(
    req: Request<Body>,
    proxy_addr: SocketAddr,
) -> Result<Response<Body>, hyper::Error> {
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("socks5://{}", proxy_addr)).unwrap())
        .build()
        .unwrap();

    let full_url = format!(
        "http://{}{}",
        req.uri().host().unwrap(),
        req.uri().path_and_query().map_or("", |pq| pq.as_str())
    );
    let response = client
        .post(&full_url)
        .body(
            hyper::body::to_bytes(req.into_body())
                .await
                .unwrap()
                .to_vec(),
        )
        .send()
        .await
        .unwrap();

    let status = response.status();
    let headers = response.headers().clone();
    let body = hyper::Body::from(response.bytes().await.unwrap().to_vec());

    let mut hyper_response = Response::new(body);
    *hyper_response.status_mut() = StatusCode::from_u16(status.as_u16()).unwrap();
    *hyper_response.headers_mut() = headers.clone();

    Ok(hyper_response)
}

async fn handle_request(
    req: Request<Body>,
    proxy_addr: SocketAddr,
) -> Result<Response<Body>, hyper::Error> {
    match *req.method() {
        Method::CONNECT => {
            let uri = req.uri().clone();
            info!("Handling CONNECT request for {}", uri);

            let host = req.uri().authority().unwrap().to_string();

            tokio::spawn(async move {
                if let Ok(upgraded) = hyper::upgrade::on(req).await {
                    match Socks5Stream::connect(proxy_addr, host).await {
                        Ok(mut proxy_stream) => {
                            let (mut client_reader, mut client_writer) = tokio::io::split(upgraded);
                            let (mut server_reader, mut server_writer) = proxy_stream.split();

                            let client_to_server =
                                tokio::io::copy(&mut client_reader, &mut server_writer);

                            let server_to_client =
                                tokio::io::copy(&mut server_reader, &mut client_writer);

                            match tokio::try_join!(client_to_server, server_to_client) {
                                Ok((_, _)) => info!("Tunnel closed successfully for {}", uri),
                                Err(e) => error!("Error in tunneling for {}: {}", uri, e),
                            }
                        }
                        Err(e) => error!(
                            "Failed to connect to target through SOCKS5 for {}: {}",
                            uri, e
                        ),
                    }
                }
            });

            Ok(Response::builder().status(200).body(Body::empty()).unwrap())
        }
        Method::POST => proxy_http_through_socks5(req, proxy_addr).await,
        Method::GET => proxy_http_through_socks5(req, proxy_addr).await,
        _ => {
            warn!("Received unsupported request method");
            Ok(Response::new(Body::from("Method Not Allowed")))
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let addr = ([127, 0, 0, 1], 3129).into();
    let proxy_addr = "127.0.0.1:12345"
        .parse::<SocketAddr>()
        .expect("Invalid SOCKS5 proxy address");

    let make_svc = make_service_fn(move |_conn| async move {
        Ok::<_, Infallible>(service_fn(move |req| handle_request(req, proxy_addr)))
    });

    let server = Server::bind(&addr).serve(make_svc);

    info!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        error!("server error: {}", e);
    }
}
