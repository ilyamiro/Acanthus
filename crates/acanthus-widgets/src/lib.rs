pub mod button;
pub mod choice;
pub mod component;
pub mod dialog;
pub mod dropdown;
pub mod marquee;
pub mod menu;
pub mod popup_frame;
pub mod profiler;
pub mod progress;
pub mod screencopy;
pub mod scrollbar;
pub mod search_field;
pub mod selector;
pub mod slider;
pub mod switch;
pub mod tabs;
pub mod text_area;
pub mod timing;
pub mod toggle;
pub mod tooltip;
pub mod wallpaper_blur;
pub mod wheel;

/// A registry holding every widget that follows the component contract.
pub fn standard_registry() -> component::Registry {
    let mut registry = component::Registry::new();
    registry.register(slider::SliderComponent);
    registry.register(choice::CheckboxComponent);
    registry.register(dialog::DialogComponent);
    registry.register(choice::RadioGroupComponent);
    registry.register(toggle::ToggleComponent);
    registry.register(tooltip::TooltipComponent);
    registry.register(button::IconButtonComponent);
    registry.register(marquee::MarqueeComponent);
    registry.register(switch::SwitchComponent);
    registry.register(tabs::TabsComponent);
    registry.register(dropdown::DropdownComponent);
    registry.register(popup_frame::PopupFrameComponent);
    registry.register(progress::ProgressBarComponent);
    registry.register(progress::SpinnerComponent);
    registry.register(scrollbar::ScrollBarComponent);
    registry.register(search_field::SearchFieldComponent);
    registry.register(selector::NumberSelectorComponent);
    registry.register(selector::TimeSelectorComponent);
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
                "Checkbox",
                "Dialog",
                "Dropdown",
                "IconButton",
                "Marquee",
                "NumberSelector",
                "PopupFrame",
                "ProgressBar",
                "RadioGroup",
                "ScrollBar",
                "SearchField",
                "Slider",
                "Spinner",
                "Switch",
                "Tabs",
                "TextArea",
                "TimeSelector",
                "Toggle",
                "Tooltip",
                "WallpaperBlur"
            ]
        );
    }
}
