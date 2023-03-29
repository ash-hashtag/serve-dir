use std::{convert::Infallible, net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc, time::UNIX_EPOCH};

use hyper::{
    header::CONTENT_TYPE,
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Method, Request, Response, Server,
};

fn print_usage() {
    println!("usage: serve-dir [directory_path] ...[options]\nset host: --host='127.0.0.1' or -h='127.0.0.1'\nset port: --port=8080 or -p=8080\nset header: --header=x-custom-header:x-custom-value or -H=x-custom-header:x-custom-value\nremove default headers([access-control-allow-origin:*]): --no-default-headers\nhelp: --help");
}

trait Update<T> {
    fn update(&mut self, value: T) -> bool;
}

impl Update<(String, String)> for Vec<(String, String)> {
    fn update(&mut self, value: (String, String)) -> bool {
        if let Some(index) = self.iter().position(|x| x.0 == value.0) {
            self[index].1 = value.1;
            true
        } else {
            self.push(value);
            false
        }
    }
}

struct SharedData {
    headers: Vec<(String, String)>,
    directory_path: String,
    not_found_file_path: Option<String>,
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);

    let mut directory_path = args.next().expect("Not Enough Arguments");
    if directory_path == "--help" {
        print_usage();
        return;
    }
    if PathBuf::from_str(&directory_path).is_err() {
        eprintln!("Invalid Directory Path");
        return;
    }
    if !directory_path.ends_with('/') && !directory_path.ends_with('\\') {
        directory_path.push('/');
    }
    let mut headers = Vec::<(String, String)>::with_capacity(10);
    let mut host: [u8; 4] = [127, 0, 0, 1];
    let mut is_host_filled = false;
    let mut port: u16 = 8080;
    let mut is_port_filled = false;
    let mut no_default_headers = false;

    let mut not_found_file_path: Option<String> = None;

    for arg in args {
        if arg == "--help" {
            print_usage();
            return;
        }
        if !is_host_filled {
            if arg.starts_with("--host=") {
                let host_addr = &arg[7..];
                let mut i = 0;
                for val in host_addr.split('.') {
                    host[i] = val.parse().expect("Host Address is invalid");
                    i += 1;
                }
                is_host_filled = true;
            } else if arg.starts_with("-h=") {
                let host_addr = &arg[3..];
                let mut i = 0;
                for val in host_addr.split('.') {
                    host[i] = val.parse().expect("Host Address is invalid");
                    i += 1;
                }
                is_host_filled = true;
            }
        }
        if !is_port_filled {
            if arg.starts_with("--port=") {
                let port_str = &arg[7..];
                port = port_str.parse().expect("Port is Invalid");
                is_port_filled = true;
            } else if arg.starts_with("-p=") {
                let port_str = &arg[3..];
                port = port_str.parse().expect("Port is Invalid");
                is_port_filled = true;
            }
        }
        if not_found_file_path.is_none() {
            if arg.starts_with("--404=") {
                not_found_file_path = Some(String::from(&arg[6..]));
            }
        }
        if arg.starts_with("--header=") {
            let header_str = &arg[9..];
            let (key, value) = header_str.split_at(header_str.find(':').expect("Invalid Header"));
            headers.update((String::from(key), String::from(value)));
        } else if arg.starts_with("-H") {
            let header_str = &arg[3..];
            let (key, value) = header_str.split_at(header_str.find(':').expect("Invalid Header"));
            headers.update((String::from(key), String::from(value)));
        } else if arg == "--no-default-headers" {
            no_default_headers = true;
        }
    }
    if !no_default_headers {
        headers.push((
            String::from("access-control-allow-origin"),
            String::from("*"),
        ));
    }

    let addr = SocketAddr::from((host, port));
    println!("Serving {} at {:?}", directory_path, addr);

    let shared_data = Arc::new(SharedData {
        headers,
        directory_path,
        not_found_file_path,
    });
    let make_service = make_service_fn(move |_: &AddrStream| {
        let data = shared_data.clone();
        async move { Ok::<_, Infallible>(service_fn(move |req| request_handler(req, data.clone()))) }
    });

    let server = Server::bind(&addr).serve(make_service);

    if let Err(e) = server.await {
        eprintln!("server initialization error {}", e);
    }
}

async fn request_handler(
    request: Request<Body>,
    shared_data: Arc<SharedData>,
) -> Result<Response<Body>, Infallible> {
    let mut response_builder = Response::builder();
    for (key, value) in &shared_data.headers {
        response_builder = response_builder.header(key, value);
    }

    let uri = request.uri();
    let time_of_request = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(std::time::Duration::default()).as_millis();
    match request.method() {

        &Method::GET => {
            let mut uri_path = &uri.path()[1..];
            if uri_path.is_empty() {
                uri_path = "index.html";
            } else {
                if uri_path.starts_with('.') {
                    println!("{}: [403] [GET] {} requested invalid path", time_of_request, uri);
                    return Ok(response_builder
                        .status(403)
                        .body(Body::from("Invalid Path"))
                        .unwrap());
                }
            }
            let file_path = PathBuf::from(format!("{}{}", shared_data.directory_path, uri_path));

            if file_path.is_file() {
                match tokio::fs::read(file_path).await {
                    Ok(body) => {
                        println!("{}: [200] [GET] {} requested file path",time_of_request, uri);
                        return Ok(response_builder.body(Body::from(body)).unwrap());
                    }
                    Err(err) => {
                        println!("{}: [500] [GET] {} {} ", time_of_request, uri, err);
                        return Ok(response_builder
                            .status(500)
                            .body(Body::from("Something Went Wrong :("))
                            .unwrap());
                    }
                };
            }
        }
        &Method::OPTIONS => {
            println!("{}: [200] [OPTIONS] {}",time_of_request, uri);
            return Ok(response_builder.body(Body::empty()).unwrap());
        }
        _ => {}
    };

    let (body, is_from_file) = not_found_body(&shared_data.not_found_file_path).await;

    let response = response_builder
        .header(
            CONTENT_TYPE,
            if is_from_file {
                "text/html"
            } else {
                "plain/text"
            },
        )
        .status(404)
        .body(body)
        .unwrap();
    println!("{}: [404] [GET] {} requested address not found",time_of_request, uri);
    return Ok(response);
}

async fn not_found_body(path: &Option<String>) -> (Body, bool) {
    const NOT_FOUND: &str = "404 Not Found";
    if let Some(val) = path {
        if let Ok(data) = tokio::fs::read(val).await {
            return (Body::from(data), true);
        }
    }
    return (Body::from(NOT_FOUND), false);
}
