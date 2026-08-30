//! Deterministic roster source for local product demos.
//!
//! This module is compiled only with `--features demo-roster`. It deliberately
//! emits the normal Discord event type so the demo cannot bypass daemon state
//! transitions or the overlay rendering path.

use std::time::Duration;

use futures_util::SinkExt;
use hyprlay_core::domain::ConnectionStatus;
use iced::stream;

use super::Message;
use super::adapters::discord::DiscordEvent;
use super::adapters::discord::Participant;

const DEFAULT_COUNT: usize = 4;
const MAX_COUNT: usize = 8;

pub(super) fn enabled() -> bool {
    count().is_some()
}

fn count() -> Option<usize> {
    std::env::var("HYPRLAY_DEMO_ROSTER")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&value| value > 0)
}

fn participants(count: usize, tick: usize) -> Vec<Participant> {
    (0..count)
        .map(|index| Participant {
            id: format!("demo-{}", index + 1),
            name: format!("Demo User {}", index + 1),
            avatar_hash: None,
            // The second participant is permanently muted and never speaks.
            speaking: index != 1 && (tick + index) % 5 < 2,
            self_mute: index == 1,
            self_deaf: false,
            server_mute: index == 2,
            server_deaf: false,
        })
        .collect()
}

fn roster(count: usize, tick: usize) -> Vec<DiscordEvent> {
    vec![
        DiscordEvent::Status(ConnectionStatus::Connected),
        DiscordEvent::Channel(Some("Demo Voice Channel".to_string())),
        DiscordEvent::Me("demo-1".to_string()),
        DiscordEvent::Participants(participants(count, tick)),
    ]
}

pub(super) fn subscription() -> impl futures_util::Stream<Item = Message> {
    stream::channel(
        16,
        |mut sender: iced::futures::channel::mpsc::Sender<Message>| async move {
            let count = count().unwrap_or(DEFAULT_COUNT).min(MAX_COUNT);
            let mut tick = 0;
            for event in roster(count, tick) {
                let _ = sender.send(Message::Discord(event)).await;
            }

            let mut interval = tokio::time::interval(Duration::from_millis(900));
            loop {
                interval.tick().await;
                tick += 1;
                let _ = sender
                    .send(Message::Discord(DiscordEvent::Participants(participants(
                        count, tick,
                    ))))
                    .await;
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_has_stable_ids_and_visible_mute_badges() {
        let users = participants(4, 0);
        assert_eq!(
            users
                .iter()
                .map(|user| user.id.as_str())
                .collect::<Vec<_>>(),
            ["demo-1", "demo-2", "demo-3", "demo-4",]
        );
        assert!(users[1].muted());
        assert!(users[2].muted());
        assert!(!users[1].speaking);
    }

    #[test]
    fn speaking_states_cycle_without_muted_user() {
        let first = participants(4, 0);
        let later = participants(4, 3);
        assert_ne!(first[0].speaking, later[0].speaking);
        assert!(!later[1].speaking);
    }
}
