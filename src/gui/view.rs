//! View layer: the window composition — header (title, search, global
//! actions), sidebar (section anchors + shortcut cheat-sheet), status bar
//! (unsaved marker, daemon toggle, last reply) — around the section pages
//! that `fields` renders.

use hyprlay_core::status::StatusFields;
use iced::Alignment;
use iced::Element;
use iced::Length;
use iced::widget::button;
use iced::widget::column;
use iced::widget::container;
use iced::widget::row;
use iced::widget::text;
use iced::widget::text_input;

use super::Gui;
use super::Message;
use super::fields::Section;
use super::fields::search_page;
use super::fields::settings_page;
use super::scroll::widget_id;
use super::theme::AMBER;
use super::theme::BRIGHT;
use super::theme::HEADER_BG;
use super::theme::MUTED;
use super::theme::REPLY_GREEN;
use super::theme::SIDEBAR_BG;
use super::theme::nav_style;
use super::theme::panel;
use super::theme::plain_style;
use super::theme::primary_style;

pub(super) fn view(gui: &Gui) -> Element<'_, Message> {
    let content = if gui.search.trim().is_empty() {
        settings_page(gui)
    } else {
        search_page(gui)
    };

    column![
        header(gui),
        container(row![sidebar(gui), content]).height(Length::Fill),
        status_bar(gui),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Title, search box, and global actions on the darkest strip.
fn header(gui: &Gui) -> Element<'_, Message> {
    // "Clear changes" only does something while the runtime config differs
    // from disk; a disabled press target communicates that at a glance.
    let mut clear = button(text("Clear changes")).style(plain_style());
    if gui.dirty {
        clear = clear.on_press(Message::ClearChanges);
    }
    container(
        row![
            text("hyprlay").size(14).color(BRIGHT),
            text_input("Search settings…  Ctrl+F", &gui.search)
                .id(widget_id())
                .on_input(Message::Search)
                .size(13)
                .padding([4, 8]),
            clear,
            button(text("Reset all"))
                .on_press(Message::ResetAll)
                .style(plain_style()),
            button(text("Save"))
                .on_press(Message::Save)
                .style(primary_style(gui.dirty)),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .style(panel(HEADER_BG))
    .into()
}

/// Section navigation plus the shortcut cheat-sheet.
fn sidebar(gui: &Gui) -> Element<'_, Message> {
    let mut nav = column![].spacing(4);
    for (i, s) in Section::ALL.iter().enumerate() {
        let selected = gui.section == *s && gui.search.trim().is_empty();
        nav = nav.push(
            button(
                row![
                    text(s.name().to_string()).size(13),
                    iced::widget::Space::new().width(Length::Fill),
                    text(format!("Ctrl+{}", i + 1)).size(9).color(MUTED),
                ]
                .align_y(Alignment::Center)
                .width(Length::Fill),
            )
            .on_press(Message::Navigate(*s))
            .width(Length::Fill)
            .style(nav_style(selected)),
        );
    }
    let hints =
        "\nCtrl+S    save\nCtrl+R    reset section\nCtrl+F    search\nEsc       clear search";
    let col = column![
        nav,
        iced::widget::Space::new().height(Length::Fill),
        text(format!("shortcuts{hints}")).size(10).color(MUTED),
    ]
    .spacing(8);
    container(col)
        .width(Length::Fixed(160.0))
        .height(Length::Fill)
        .padding([10, 8])
        .style(panel(SIDEBAR_BG))
        .into()
}

fn status_bar(gui: &Gui) -> Element<'_, Message> {
    let unsaved = if gui.dirty {
        text("● unsaved").size(11).color(AMBER)
    } else {
        text("").size(11)
    };
    container(
        row![
            unsaved,
            daemon_toggle(gui),
            text("daemon").size(10).color(MUTED),
            text(brief_status(gui.daemon_state.text())).size(11),
            iced::widget::Space::new().width(Length::Fill),
            text("last change").size(10).color(MUTED),
            text(gui.last_reply.clone()).size(11).color(REPLY_GREEN),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([6, 12])
    .width(Length::Fill)
    .style(panel(HEADER_BG))
    .into()
}

/// Bottom-left Start/Stop control. Disabled (no press target) while no
/// probe has answered yet, mirroring how "Clear changes" disables itself.
fn daemon_toggle(gui: &Gui) -> Element<'_, Message> {
    let mut toggle = button(text(gui.daemon_state.label()).size(11)).style(plain_style());
    if gui.daemon_state.toggle().is_some() {
        toggle = toggle.on_press(Message::ToggleDaemon);
    }
    toggle.into()
}

/// "status=connected channel=ngobrol 3 participants=2 …" →
/// "connected · ngobrol 3". Parsing goes through the shared
/// [`StatusFields`]; channel names may contain spaces, which its
/// marker-slice handles.
fn brief_status(full: &str) -> String {
    match StatusFields::parse_wire(full) {
        Some(fields) if !fields.channel.is_empty() => {
            format!("{} · {}", fields.status_word, fields.channel)
        }
        Some(fields) => fields.status_word.to_string(),
        None => full.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_status_keeps_multiword_channel_names_intact() {
        let full = "status=connected channel=ngobrol 3 participants=2 rtl=on monitor=eDP-1";
        assert_eq!(brief_status(full), "connected · ngobrol 3");
    }

    #[test]
    fn brief_status_without_channel_falls_back_to_connection_word() {
        assert_eq!(brief_status("status=disconnected"), "disconnected");
        assert_eq!(brief_status("connecting…"), "connecting…");
        // Empty channel value (malformed Discord payload only): the old
        // code printed "connected · " with a trailing separator; the new
        // code drops it. Pinned here so the tightening stays deliberate.
        assert_eq!(
            brief_status("status=connected channel= participants=0"),
            "connected"
        );
    }
}
