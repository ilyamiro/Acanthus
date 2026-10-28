use smithay_client_toolkit::output::OutputInfo as RawOutputInfo;
use wayland_client::protocol::wl_output::WlOutput;

/// A connected output.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    /// Stable id of the output.
    pub id: u32,
    /// Connector name such as `DP-1`.
    pub name: String,
    /// Manufacturer.
    pub make: String,
    /// Model.
    pub model: String,
    /// Position in the compositor space in logical pixels.
    pub position: (i32, i32),
    /// Size in logical pixels.
    pub size: (i32, i32),
    /// Integer scale factor.
    pub scale: i32,
    /// The protocol object, for placing surfaces on this output.
    pub output: WlOutput,
}

impl OutputInfo {
    pub(crate) fn from_raw(raw: &RawOutputInfo, output: WlOutput) -> Self {
        let scale = raw.scale_factor.max(1);
        let size = raw.logical_size.unwrap_or_else(|| {
            raw.modes.iter().find(|m| m.current).map_or((0, 0), |m| (m.dimensions.0 / scale, m.dimensions.1 / scale))
        });
        OutputInfo {
            id: raw.id,
            name: raw.name.clone().unwrap_or_default(),
            make: raw.make.clone(),
            model: raw.model.clone(),
            position: raw.logical_position.unwrap_or(raw.location),
            size,
            scale,
            output,
        }
    }
}
