//! The core's events, brought to the UI thread. The core emits from any
//! thread into a channel; the [`EventHub`] re-emits each on the foreground,
//! where views subscribe to it (`cx.subscribe(&EventHub::global(cx), ..)`).

use dbforge_core::{Event, Events};
use gpui::{App, AppContext, Entity, EventEmitter, Global};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The core's side: sends each event to the hub. Events sent before the
/// window exists wait in the channel.
pub struct ChannelEvents(UnboundedSender<Event>);

impl Events for ChannelEvents {
    fn emit(&self, event: Event) {
        // Fails only once the app is quitting and the hub is gone.
        let _ = self.0.send(event);
    }
}

/// A channel for the core's events: the core's end and the hub's.
pub fn channel() -> (ChannelEvents, UnboundedReceiver<Event>) {
    let (sender, receiver) = unbounded_channel();
    (ChannelEvents(sender), receiver)
}

/// The core's events on the UI thread, for views to subscribe to.
pub struct EventHub;

impl EventEmitter<Event> for EventHub {}

struct GlobalHub(Entity<EventHub>);

impl Global for GlobalHub {}

impl EventHub {
    /// Creates the hub and forwards `events` to it for as long as the app runs.
    pub fn start(mut events: UnboundedReceiver<Event>, cx: &mut App) {
        let hub = cx.new(|_| EventHub);
        cx.set_global(GlobalHub(hub.clone()));
        cx.spawn(async move |cx| {
            while let Some(event) = events.recv().await {
                if hub.update(cx, |_, cx| cx.emit(event)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn global(cx: &App) -> Entity<EventHub> {
        cx.global::<GlobalHub>().0.clone()
    }
}
