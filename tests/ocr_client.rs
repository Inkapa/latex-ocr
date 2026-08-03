//! Integration test for the HTTP OCR backend against a real local server.

use image::{Rgba, RgbaImage};
use latex_ocr::ocr::{HttpOcrBackend, OcrBackend};

/// Tiny HTTP server that answers a single POST with the given body.
fn serve_once(
    body: &'static [u8],
    content_type: &'static str,
) -> (u16, std::thread::JoinHandle<()>) {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("start test server");
    let port = server.server_addr().to_ip().expect("tcp addr").port();
    let handle = std::thread::spawn(move || {
        let mut request = server.recv().expect("receive request");
        let mut buf = Vec::new();
        request
            .as_reader()
            .read_to_end(&mut buf)
            .expect("read body");
        let response = tiny_http::Response::from_data(body).with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type).unwrap(),
        );
        request.respond(response).expect("respond");
    });
    (port, handle)
}

fn sample_image() -> RgbaImage {
    RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]))
}

#[test]
fn recognizes_from_pix2tex_style_response() {
    let (port, server) = serve_once(b"{\"pred\": \"x^2 + y^2\"}", "application/json");
    let backend = HttpOcrBackend::new(format!("http://127.0.0.1:{port}/"));
    let latex = backend.recognize(&sample_image()).expect("recognize");
    assert_eq!(latex, "x^2 + y^2");
    server.join().unwrap();
}

#[test]
fn handles_plain_text_response() {
    let (port, server) = serve_once(b"\\frac{1}{2}", "text/plain");
    let backend = HttpOcrBackend::new(format!("http://127.0.0.1:{port}"));
    let latex = backend.recognize(&sample_image()).expect("recognize");
    assert_eq!(latex, "\\frac{1}{2}");
    server.join().unwrap();
}

#[test]
fn reports_unreachable_server() {
    // Port 1 is never bound on normal systems.
    let backend = HttpOcrBackend::new("http://127.0.0.1:1");
    let err = backend.recognize(&sample_image()).unwrap_err();
    assert!(err.message.contains("unreachable"));
}

#[test]
fn reports_http_error_status() {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("start test server");
    let port = server.server_addr().to_ip().expect("tcp addr").port();
    let handle = std::thread::spawn(move || {
        let request = server.recv().expect("receive request");
        let response = tiny_http::Response::from_data(&b"oops"[..]).with_status_code(500);
        request.respond(response).expect("respond");
    });
    let backend = HttpOcrBackend::new(format!("http://127.0.0.1:{port}"));
    let err = backend.recognize(&sample_image()).unwrap_err();
    assert!(err.message.contains("500"));
    handle.join().unwrap();
}
