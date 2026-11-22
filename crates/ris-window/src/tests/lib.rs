//! What the window serves, asked for as the web view asks, with no
//! window opened.

use super::*;

fn get(uri: &str) -> Request<Vec<u8>> {
    Request::builder().uri(uri).body(Vec::new()).unwrap()
}

#[test]
fn the_address_the_web_view_opens_is_served_the_page_whole() {
    let page = "<!doctype html><title>the map</title><p>caf\u{e9}</p>";
    let response = respond(page, &get(URL));
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[CONTENT_TYPE],
        "text/html; charset=utf-8",
        "the page is read as a page, in the encoding it is written in"
    );
    assert_eq!(response.body().as_ref(), page.as_bytes());
}

#[test]
fn the_address_is_under_the_scheme_the_window_registers() {
    // `show` registers SCHEME and opens URL: were they to part, the web
    // view would ask something else for the page.
    let uri: wry::http::Uri = URL.parse().unwrap();
    assert_eq!(uri.scheme_str(), Some(SCHEME));
    assert_eq!(uri.path(), "/");
}

#[test]
fn nothing_else_is_served() {
    for uri in ["ris://localhost/map.html", "ris://localhost/d3.js"] {
        let response = respond("the page", &get(uri));
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_ne!(response.body().as_ref(), b"the page", "{uri}");
    }
}
