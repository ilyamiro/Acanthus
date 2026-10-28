use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::sync::Mutex;

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Proxy, QueueHandle};

use crate::app::State;
use crate::clipboard::offered_mimes;

pub(crate) trait DataControl {
    fn has_device(&self) -> bool;
    fn ensure_device(&mut self, seat: &WlSeat, qh: &QueueHandle<State>);
    fn offer(&mut self, text: &str, qh: &QueueHandle<State>);
}

pub(crate) fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Option<Box<dyn DataControl>> {
    if let Some(control) = ext::Control::bind(globals, qh) {
        return Some(Box::new(control));
    }
    wlr::Control::bind(globals, qh).map(|c| Box::new(c) as Box<dyn DataControl>)
}

fn open_pipe(receive: impl FnOnce(std::os::fd::BorrowedFd<'_>)) -> io::Result<File> {
    let (reader, writer) = io::pipe()?;
    receive(writer.as_fd());
    drop(writer);
    Ok(File::from(std::os::fd::OwnedFd::from(reader)))
}

fn write_in_background(fd: std::os::fd::OwnedFd, text: String) {
    std::thread::spawn(move || {
        let _ = File::from(fd).write_all(text.as_bytes());
    });
}

pub(crate) struct ManagerData;

pub(crate) struct OfferData {
    mimes: Mutex<Vec<String>>,
}

pub(crate) struct SourceData {
    text: String,
}

pub(crate) struct DeviceData;

macro_rules! data_control {
    () => {
        pub(super) struct Control {
            manager: Manager,
            device: Option<Device>,
            source: Option<Source>,
        }

        impl Control {
            pub fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Option<Self> {
                let manager = globals.bind(qh, 1..=1, ManagerData).ok()?;
                Some(Control { manager, device: None, source: None })
            }
        }

        impl DataControl for Control {
            fn has_device(&self) -> bool {
                self.device.is_some()
            }

            fn ensure_device(&mut self, seat: &WlSeat, qh: &QueueHandle<State>) {
                if self.device.is_none() {
                    self.device = Some(self.manager.get_data_device(seat, qh, DeviceData));
                }
            }

            fn offer(&mut self, text: &str, qh: &QueueHandle<State>) {
                let Some(device) = self.device.as_ref() else { return };
                let source = self.manager.create_data_source(qh, SourceData { text: text.to_owned() });
                for mime in offered_mimes() {
                    source.offer(mime.to_owned());
                }
                device.set_selection(Some(&source));
                if let Some(old) = self.source.replace(source)
                    && old.is_alive()
                {
                    old.destroy();
                }
            }
        }

        impl Dispatch2<Manager, State> for ManagerData {
            fn event(&self, _: &mut State, _: &Manager, _: manager::Event, _: &Connection, _: &QueueHandle<State>) {}
        }

        impl Dispatch2<Offer, State> for OfferData {
            fn event(&self, _: &mut State, _: &Offer, event: offer::Event, _: &Connection, _: &QueueHandle<State>) {
                if let offer::Event::Offer { mime_type } = event
                    && let Ok(mut mimes) = self.mimes.lock()
                {
                    mimes.push(mime_type);
                }
            }
        }

        impl Dispatch2<Source, State> for SourceData {
            fn event(
                &self,
                _: &mut State,
                source: &Source,
                event: source::Event,
                _: &Connection,
                _: &QueueHandle<State>,
            ) {
                match event {
                    source::Event::Send { fd, .. } => write_in_background(fd, self.text.clone()),
                    source::Event::Cancelled if source.is_alive() => source.destroy(),
                    _ => {}
                }
            }
        }

        impl Dispatch2<Device, State> for DeviceData {
            wayland_client::event_created_child!(State, Device, [
                0 => (Offer, OfferData { mimes: Mutex::new(Vec::new()) })
            ]);

            fn event(
                &self,
                state: &mut State,
                _: &Device,
                event: device::Event,
                conn: &Connection,
                _: &QueueHandle<State>,
            ) {
                match event {
                    device::Event::Selection { id: Some(offer) } => {
                        let mimes = offer.data::<OfferData>().and_then(|d| d.mimes.lock().ok().map(|m| m.clone())).unwrap_or_default();
                        let receive = |mime: &str| {
                            let result = open_pipe(|fd| offer.receive(mime.to_owned(), fd));
                            let _ = conn.flush();
                            result
                        };
                        state.on_data_selection(Some(mimes), &receive);
                        offer.destroy();
                    }
                    device::Event::Selection { id: None } => {
                        state.on_data_selection(None, &|_| Err(io::ErrorKind::NotFound.into()));
                    }
                    device::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
                    _ => {}
                }
            }
        }
    };
}

mod ext {
    use super::*;
    use device::ExtDataControlDeviceV1 as Device;
    use manager::ExtDataControlManagerV1 as Manager;
    use offer::ExtDataControlOfferV1 as Offer;
    use source::ExtDataControlSourceV1 as Source;
    use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1 as device;
    use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1 as manager;
    use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1 as offer;
    use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1 as source;

    data_control!();
}

mod wlr {
    use super::*;
    use device::ZwlrDataControlDeviceV1 as Device;
    use manager::ZwlrDataControlManagerV1 as Manager;
    use offer::ZwlrDataControlOfferV1 as Offer;
    use source::ZwlrDataControlSourceV1 as Source;
    use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_device_v1 as device;
    use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1 as manager;
    use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_offer_v1 as offer;
    use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_source_v1 as source;

    data_control!();
}
