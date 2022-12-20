use std::fmt::Display;

use inquire::ui::{Attributes, RenderConfig, StyleSheet, Styled};

use super::colors;

pub trait InquireExt {
    fn with_flox_theme(self) -> Self;
}

impl<T> InquireExt for inquire::Select<'_, T>
where
    T: Display,
{
    fn with_flox_theme(self) -> Self {
        self.with_render_config(flox_theme())
    }
}

fn flox_theme() -> RenderConfig {
    let mut render_config = RenderConfig::default_colored();
    render_config.answered_prompt_prefix =
        Styled::new(">").with_fg(colors::LIGHT_PEACH.to_inquire());
    render_config.highlighted_option_prefix =
        Styled::new(">").with_fg(colors::LIGHT_PEACH.to_inquire());
    render_config.prompt_prefix = Styled::new("?").with_fg(colors::LIGHT_PEACH.to_inquire());
    render_config.prompt = StyleSheet::new().with_attr(Attributes::BOLD);
    render_config.help_message = Styled::new("")
        .with_fg(colors::LIGHT_BLUE.to_inquire())
        .style;
    render_config.answer = Styled::new("")
        .with_fg(colors::LIGHT_PEACH.to_inquire())
        .style;
    render_config
}
