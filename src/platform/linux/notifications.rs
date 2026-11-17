//! Desktop notification server (ADR-0007): owns `org.freedesktop.Notifications` on the session bus,
//! turns each `Notify` into `Event::Notification` and sends back the state's answers as D-Bus signals.
//!
//! If another server already owns the name, a warning is printed and hidebar carries on without notifications.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use zbus::blocking::connection;
use zbus::interface;
use zbus::names::BusName;
use zbus::zvariant::{OwnedValue, Value};

use crate::event::{Event, Notification, NotificationIcon, Timeout, Urgency};
use crate::state::Effect;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// Action buttons on a card (ADR-0007, design revision).
const MAX_BUTTONS: usize = 3;

type Emit = Box<dyn Fn(Event) -> bool + Send + Sync>;

struct Server {
    emit: Emit,
    next_id: AtomicU32,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id == 0 {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        } else {
            replaces_id
        };
        (self.emit)(Event::Notification(Notification {
            id,
            app: app_name,
            summary: plain_text(&summary),
            body: plain_text(&body),
            icon: icon(&app_icon, &hints),
            urgency: urgency(&hints),
            timeout: timeout(expire_timeout),
            default_action: actions.chunks(2).any(|pair| pair[0] == "default"),
            actions: buttons(&actions),
            progress: hints
                .get("value")
                .and_then(|value| i32::try_from(&**value).ok())
                .map(|value| value.clamp(0, 100) as u8),
        }));
        id
    }

    fn close_notification(&self, id: u32) {
        (self.emit)(Event::NotificationClosed(id));
    }

    fn get_capabilities(&self) -> Vec<&'static str> {
        vec!["body", "icon-static", "persistence", "actions"]
    }

    fn get_server_information(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        ("hidebar", "hidebar", env!("CARGO_PKG_VERSION"), "1.2")
    }
}

/// Starts the server in its own thread. Returns where to send the state's answers (`Effect`).
pub fn listen(emit: impl Fn(Event) -> bool + Send + Sync + 'static) -> Sender<Effect> {
    let (effects, answers) = mpsc::channel::<Effect>();
    std::thread::spawn(move || {
        let server = Server {
            emit: Box::new(emit),
            next_id: AtomicU32::new(1),
        };
        let connection = connection::Builder::session()
            .and_then(|builder| builder.name(NAME))
            .and_then(|builder| builder.serve_at(PATH, server))
            .and_then(|builder| builder.build());
        let connection = match connection {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("hidebar: no desktop notifications ({NAME}): {error}");
                return;
            }
        };
        for effect in answers {
            let sent = match effect {
                Effect::NotificationClosed { id, reason } => connection.emit_signal(
                    None::<BusName<'_>>,
                    PATH,
                    NAME,
                    "NotificationClosed",
                    &(id, reason as u32),
                ),
                Effect::ActionInvoked { id, key } => connection.emit_signal(
                    None::<BusName<'_>>,
                    PATH,
                    NAME,
                    "ActionInvoked",
                    &(id, key.as_str()),
                ),
            };
            if let Err(error) = sent {
                eprintln!("hidebar: notification signal: {error}");
            }
        }
    });
    effects
}

/// `actions` comes as key, label, key, label...; `default` is the click on the card, not a button.
fn buttons(actions: &[String]) -> Vec<(String, String)> {
    actions
        .chunks_exact(2)
        .filter(|pair| pair[0] != "default")
        .take(MAX_BUTTONS)
        .map(|pair| (pair[0].clone(), plain_text(&pair[1])))
        .collect()
}

fn timeout(expire_timeout: i32) -> Timeout {
    match expire_timeout {
        0 => Timeout::Never,
        milliseconds if milliseconds > 0 => {
            Timeout::After(Duration::from_millis(milliseconds as u64))
        }
        _ => Timeout::Default,
    }
}

fn urgency(hints: &HashMap<String, OwnedValue>) -> Urgency {
    match hints
        .get("urgency")
        .and_then(|value| u8::try_from(&**value).ok())
    {
        Some(0) => Urgency::Low,
        Some(2) => Urgency::Critical,
        _ => Urgency::Normal,
    }
}

/// Image by priority (spec 1.2): `image-data`, `image-path`, then `app_icon`. Icon theme names are not looked
/// up (ADR-0007 Q4): the view draws its own icon instead.
fn icon(app_icon: &str, hints: &HashMap<String, OwnedValue>) -> Option<NotificationIcon> {
    let pixels = ["image-data", "image_data", "icon_data"]
        .iter()
        .find_map(|key| hints.get(*key))
        .and_then(pixels);
    if pixels.is_some() {
        return pixels;
    }
    let image_path = ["image-path", "image_path"]
        .iter()
        .find_map(|key| hints.get(*key))
        .and_then(|value| <&str>::try_from(&**value).ok());
    image_path
        .and_then(file)
        .or_else(|| file(app_icon))
        .map(NotificationIcon::Path)
}

/// Absolute path or `file://` URI; anything else is an icon theme name.
fn file(icon: &str) -> Option<PathBuf> {
    let path = PathBuf::from(icon.strip_prefix("file://").unwrap_or(icon));
    path.is_absolute().then_some(path)
}

/// `(iiibiiay)`: width, height, row stride, has alpha, bits per sample, channels, data → 8-bit RGBA.
fn pixels(value: &OwnedValue) -> Option<NotificationIcon> {
    let value: Value<'_> = value.try_clone().ok()?.into();
    let (width, height, stride, alpha, bits, channels, data) =
        <(i32, i32, i32, bool, i32, i32, Vec<u8>)>::try_from(value).ok()?;
    let rgba = to_rgba(width, height, stride, alpha, bits, channels, &data)?;
    Some(NotificationIcon::Pixels {
        width: width as u32,
        height: height as u32,
        rgba: rgba.into(),
    })
}

fn to_rgba(
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
    bits: i32,
    channels: i32,
    data: &[u8],
) -> Option<Vec<u8>> {
    let channels_expected = if alpha { 4 } else { 3 };
    if width <= 0 || height <= 0 || bits != 8 || channels != channels_expected {
        return None;
    }
    let (width, height, stride, channels) = (
        width as usize,
        height as usize,
        stride as usize,
        channels as usize,
    );
    if stride < width * channels || data.len() < stride * (height - 1) + width * channels {
        return None;
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in data.chunks(stride).take(height) {
        for pixel in row[..width * channels].chunks_exact(channels) {
            rgba.extend_from_slice(&pixel[..3]);
            rgba.push(if alpha { pixel[3] } else { u8::MAX });
        }
    }
    Some(rgba)
}

/// The body may carry a small HTML subset (`<b>`, `<i>`, `<a>`...): tags are dropped and entities decoded.
fn plain_text(markup: &str) -> String {
    let mut text = String::with_capacity(markup.len());
    let mut in_tag = false;
    for character in markup.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_becomes_plain_text() {
        assert_eq!(
            plain_text("<b>Hi</b> &amp; <a href=\"x\">bye</a> &lt;3"),
            "Hi & bye <3"
        );
    }

    #[test]
    fn rgb_rows_with_padding_become_rgba() {
        // 2×2 RGB with a 1-byte pad at the end of each row.
        let data = [1, 2, 3, 4, 5, 6, 0, 7, 8, 9, 10, 11, 12, 0];
        let rgba = to_rgba(2, 2, 7, false, 8, 3, &data).unwrap();
        assert_eq!(
            rgba,
            [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
        assert!(
            to_rgba(2, 2, 7, false, 8, 3, &data[..10]).is_none(),
            "short data"
        );
    }

    #[test]
    fn default_is_not_a_button() {
        let actions = [
            "default",
            "Open",
            "reply",
            "<b>Reply</b>",
            "read",
            "Mark read",
        ]
        .map(String::from);
        assert_eq!(
            buttons(&actions),
            [
                ("reply".to_owned(), "Reply".to_owned()),
                ("read".to_owned(), "Mark read".to_owned())
            ]
        );
    }

    #[test]
    fn timeouts_follow_the_spec() {
        assert_eq!(timeout(-1), Timeout::Default);
        assert_eq!(timeout(0), Timeout::Never);
        assert_eq!(timeout(1500), Timeout::After(Duration::from_millis(1500)));
    }

    #[test]
    fn theme_names_are_not_files() {
        assert_eq!(file("firefox"), None);
        assert_eq!(file("file:///tmp/a.png"), Some(PathBuf::from("/tmp/a.png")));
    }
}
