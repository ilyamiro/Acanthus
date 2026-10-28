use std::collections::HashMap;

use futures_lite::StreamExt;
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

use super::Shortcut;
use crate::platform::bridge::EventSink;
use crate::platform::dbus::{BusKind, portal_request_path};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST: &str = "org.freedesktop.portal.Request";

pub(super) enum Outcome {
    Bound,
    Unavailable,
}

fn token(kind: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("acanthus_{kind}_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

async fn response(
    call: impl std::future::Future<Output = zbus::Result<OwnedObjectPath>>,
    mut stream: zbus::proxy::SignalStream<'_>,
) -> zbus::Result<(u32, HashMap<String, OwnedVariant>)> {
    call.await?;
    let message = stream.next().await.ok_or_else(|| zbus::Error::Failure("portal request vanished".to_owned()))?;
    message.body().deserialize()
}

async fn request_stream<'a>(
    connection: &'a zbus::Connection,
    token: &str,
) -> zbus::Result<zbus::proxy::SignalStream<'a>> {
    let unique = connection.unique_name().map(|name| name.to_string()).unwrap_or_default();
    let path = portal_request_path(&unique, token);
    let proxy = zbus::Proxy::new(connection, SERVICE, path, REQUEST).await?;
    proxy.receive_signal("Response").await
}

pub(super) fn bind_entries(shortcuts: &[Shortcut]) -> Vec<(String, HashMap<&'static str, Variant<'static>>)> {
    shortcuts
        .iter()
        .map(|shortcut| {
            let mut options: HashMap<&'static str, Variant<'static>> = HashMap::new();
            options.insert("description", Variant::from(shortcut.description.clone()));
            options.insert("preferred_trigger", Variant::from(shortcut.chord.portal()));
            (shortcut.id.clone(), options)
        })
        .collect()
}

pub(super) async fn run(bus: BusKind, shortcuts: Vec<Shortcut>, events: EventSink<super::Event>) {
    match session(&bus, &shortcuts, &events).await {
        Ok(()) => {}
        Err(error) => {
            tracing::info!("global shortcuts portal unavailable: {error}");
            events.send(super::Event::Portal(Outcome::Unavailable));
        }
    }
}

async fn session(bus: &BusKind, shortcuts: &[Shortcut], events: &EventSink<super::Event>) -> zbus::Result<()> {
    let connection = bus.connect().await?;
    let portal = zbus::Proxy::new(&connection, SERVICE, PATH, INTERFACE).await?;
    let _version: u32 = portal.get_property("version").await?;
    let mut activated = portal.receive_signal("Activated").await?;

    let create_token = token("create");
    let session_token = token("session");
    let stream = request_stream(&connection, &create_token).await?;
    let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
    options.insert("handle_token", Variant::from(create_token.as_str()));
    options.insert("session_handle_token", Variant::from(session_token.as_str()));
    let (code, results) = response(portal.call("CreateSession", &(&options,)), stream).await?;
    if code != 0 {
        return Err(zbus::Error::Failure(format!("CreateSession answered {code}")));
    }
    let handle: String = results
        .get("session_handle")
        .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
        .ok_or_else(|| zbus::Error::Failure("CreateSession returned no session handle".to_owned()))?;
    let session_path = zbus::ObjectPath::try_from(handle.as_str())?;

    let bind_token = token("bind");
    let stream = request_stream(&connection, &bind_token).await?;
    let mut options: HashMap<&str, Variant<'_>> = HashMap::new();
    options.insert("handle_token", Variant::from(bind_token.as_str()));
    let entries = bind_entries(shortcuts);
    let (code, _) = response(portal.call("BindShortcuts", &(&session_path, &entries, "", &options)), stream).await?;
    if code != 0 {
        return Err(zbus::Error::Failure(format!("BindShortcuts answered {code}")));
    }
    events.send(super::Event::Portal(Outcome::Bound));

    while let Some(message) = activated.next().await {
        let Ok((_session, id, _timestamp, _options)): zbus::Result<(
            OwnedObjectPath,
            String,
            u64,
            HashMap<String, OwnedVariant>,
        )> = message.body().deserialize() else {
            continue;
        };
        events.send(super::Event::Activated(id));
    }
    Err(zbus::Error::Failure("portal signal stream ended".to_owned()))
}
