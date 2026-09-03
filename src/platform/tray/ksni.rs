//! Linux tray backend: a D-Bus `StatusNotifierItem` via `ksni`.
//!
//! Behaviour is byte-identical to the ticket-01 tray front: id `hyprlay-tray`,
//! title `hyprlay`, ARGB32 pixmaps, the shared menu model, the
//! `watcher_offline` warn-once, and `assume_sni_available(true)` so a missing
//! StatusNotifierWatcher host is non-fatal. The only structural change is that
//! the item state is now driven through the shared [`Tray`](crate::tray::Tray)
//! port instead of `mod.rs` holding the `ksni` impl directly.

use ksni::TrayMethods;
use tokio::sync::mpsc::UnboundedSender;

use crate::tray::icon::IconData;
use crate::tray::menu::MenuAction;
use crate::tray::menu::MenuRow;
use crate::tray::menu::TrayState;
use crate::tray::menu::build_menu;
use crate::tray::port::Tray;

/// The ksni D-Bus item state: the live [`TrayState`] plus the action channel
/// its menu activate closures push onto.
struct KsniState {
    state: TrayState,
    tx: UnboundedSender<MenuAction>,
    connected_icon: ksni::Icon,
    disconnected_icon: ksni::Icon,
}

impl ksni::Tray for KsniState {
    fn id(&self) -> String {
        "hyprlay-tray".into()
    }

    fn title(&self) -> String {
        "hyprlay".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![if self.state.up {
            self.connected_icon.clone()
        } else {
            self.disconnected_icon.clone()
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: self.icon_pixmap(),
            title: "hyprlay".into(),
            description: self.state.summary.clone(),
        }
    }

    fn menu(&self) -> Vec<ksni::menu::MenuItem<Self>> {
        build_menu(&self.state)
            .into_iter()
            .map(|row| match row {
                MenuRow::Status(label) => ksni::menu::StandardItem {
                    label,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                MenuRow::Separator => ksni::menu::MenuItem::Separator,
                MenuRow::Item { label, action } => ksni::menu::StandardItem {
                    label,
                    enabled: true,
                    // Non-blocking: hand the action to the poll loop.
                    activate: Box::new(move |tray: &mut KsniState| {
                        let _ = tray.tx.send(action);
                    }),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        // No StatusNotifierWatcher host (e.g. waybar not started yet): log
        // once and keep running idle — never fatal. A late-starting host will
        // bring the icon up on its own.
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!(
                event = "tray_watcher_offline",
                "no StatusNotifierWatcher host; tray stays idle until one appears"
            );
        }
        true
    }
}

/// The [`Tray`] port behind the `ksni` handle.
pub struct KsniTray {
    handle: ksni::Handle<KsniState>,
}

impl KsniTray {
    /// Register the `StatusNotifierItem`. `assume_sni_available` routes a
    /// missing Watcher/WontShow to `watcher_offline` instead of failing spawn,
    /// so a late host still works. A hard D-Bus error remains fatal.
    async fn spawn(
        tx: UnboundedSender<MenuAction>,
        connected: IconData,
        disconnected: IconData,
    ) -> Result<Self, ksni::Error> {
        let state = KsniState {
            state: TrayState::down(),
            tx,
            connected_icon: to_ksni_icon(&connected),
            disconnected_icon: to_ksni_icon(&disconnected),
        };
        let handle = state.assume_sni_available(true).spawn().await?;
        Ok(Self { handle })
    }
}

impl Tray for KsniTray {
    async fn update(&mut self, state: &TrayState) {
        let state = state.clone();
        self.handle.update(move |tray| tray.state = state).await;
    }

    async fn shutdown(&mut self) {
        // Dropping the handle unregisters the item; nothing to flush here.
    }
}

fn to_ksni_icon(data: &IconData) -> ksni::Icon {
    ksni::Icon {
        width: data.width as i32,
        height: data.height as i32,
        data: data.argb32(),
    }
}

/// Register the Linux tray and drive the shared poll loop on a current-thread
/// tokio runtime.
pub fn run(connected: IconData, disconnected: IconData) -> i32 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("hyprlay tray: could not start runtime");
    rt.block_on(run_async(connected, disconnected))
}

async fn run_async(connected: IconData, disconnected: IconData) -> i32 {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<MenuAction>();
    let mut tray = match KsniTray::spawn(tx, connected, disconnected).await {
        Ok(tray) => tray,
        Err(e) => {
            tracing::error!(
                event = "tray_spawn_failed",
                error = %e,
                "could not register the system tray"
            );
            return 1;
        }
    };
    crate::tray::poll_loop(&mut tray, &mut rx).await
}
