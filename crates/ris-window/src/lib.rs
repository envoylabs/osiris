//! A native window holding the system's web view, showing one page that
//! the program serves itself (D-138).
//!
//! The page reaches the web view through a scheme of the program's own:
//! the web view asks for [`URL`] and [`respond`] answers with the page,
//! so nothing listens on a port and nothing leaves the machine. Only
//! [`show`] opens a window; [`respond`] is the whole of what is served
//! and opens nothing, so a test can ask it for the page.
//!
//! The web view is the system's: WebKit on macOS, which is part of the
//! system, and WebKitGTK on Linux, where building this crate needs the
//! package `libwebkit2gtk-4.1-dev`.

use std::borrow::Cow;
use std::convert::Infallible;
use std::sync::Arc;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;
use wry::http::header::CONTENT_TYPE;
use wry::http::{Request, Response, StatusCode};

/// The scheme the page is served under.
pub const SCHEME: &str = "ris";

/// Where the web view finds the page.
pub const URL: &str = "ris://localhost/";

/// The answer to one request from the web view: the page for the root
/// path, and nothing for any other, since the page is self-contained.
pub fn respond<T>(page: &str, request: &Request<T>) -> Response<Cow<'static, [u8]>> {
    let (status, kind, body) = if request.uri().path() == "/" {
        (
            StatusCode::OK,
            "text/html; charset=utf-8",
            page.as_bytes().to_vec(),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            b"not found".to_vec(),
        )
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, kind)
        .body(Cow::Owned(body))
        .expect("a status and one fixed header make a response")
}

/// Open a window titled `title` showing `page`, and stay until it is
/// closed, which ends the process: this returns only when the window or
/// its web view could not be made, with the reason. `shown` is called
/// once both exist, before the first event is handled.
///
/// Call it on the main thread, which is where macOS takes its events.
pub fn show(title: &str, page: String, shown: impl FnOnce()) -> Result<Infallible, String> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(1400.0, 900.0))
        .build(&event_loop)
        .map_err(|e| format!("the window could not be made: {e}"))?;
    let page: Arc<str> = page.into();
    let builder = WebViewBuilder::new()
        .with_custom_protocol(SCHEME.to_owned(), move |_id, request| {
            respond(&page, &request)
        })
        .with_url(URL);
    #[cfg(any(target_os = "macos", windows))]
    let webview = builder.build(&window);
    // Elsewhere (Linux) the web view is a GTK widget, placed in the
    // window's box.
    #[cfg(not(any(target_os = "macos", windows)))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => return Err("the window has no box to hold the web view".to_owned()),
        }
    };
    let webview = webview.map_err(|e| format!("the web view could not be made: {e}"))?;
    shown();
    event_loop.run(move |event, _, control_flow| {
        // Both live as long as the loop, which owns them from here.
        let _ = (&window, &webview);
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    })
}

#[cfg(test)]
#[path = "tests/lib.rs"]
mod tests;
