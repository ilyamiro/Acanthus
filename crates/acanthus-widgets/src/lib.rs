pub mod button;
pub mod component;
pub mod dropdown;
pub mod marquee;
pub mod menu;
pub mod profiler;
pub mod progress;
pub mod screencopy;
pub mod scrollbar;
pub mod search_field;
pub mod slider;
pub mod switch;
pub mod text_area;
pub mod timing;
pub mod toggle;
pub mod wallpaper_blur;
pub mod wheel;

/// A registry holding every widget that follows the component contract.
pub fn standard_registry() -> component::Registry {
    let mut registry = component::Registry::new();
    registry.register(slider::SliderComponent);
    registry.register(toggle::ToggleComponent);
    registry.register(button::IconButtonComponent);
    registry.register(marquee::MarqueeComponent);
    registry.register(switch::SwitchComponent);
    registry.register(dropdown::DropdownComponent);
    registry.register(progress::ProgressBarComponent);
    registry.register(progress::SpinnerComponent);
    registry.register(scrollbar::ScrollBarComponent);
    registry.register(search_field::SearchFieldComponent);
    registry.register(text_area::TextAreaComponent);
    registry.register(wallpaper_blur::WallpaperBlurComponent);
    registry
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_standard_registry_lists_its_components_sorted() {
        assert_eq!(
            super::standard_registry().names(),
            vec![
                "Dropdown",
                "IconButton",
                "Marquee",
                "ProgressBar",
                "ScrollBar",
                "SearchField",
                "Slider",
                "Spinner",
                "Switch",
                "TextArea",
                "Toggle",
                "WallpaperBlur"
            ]
        );
    }
}
