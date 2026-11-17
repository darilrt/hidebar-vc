//! Desktop notifications on screen (ADR-0007): a stack of up to 3 cards (6 once expanded), critical ones first,
//! the rest waiting behind a "N more" pill. Each card stretches in and folds back like the panels (ADR-0006),
//! expires on its own time (paused under the cursor) and answers the app through `Effect`.

use std::time::{Duration, Instant};

use super::{Frame, Motion};
use crate::event::{Notification, Timeout, Urgency};

/// How long a notification stays when the app lets the server decide, and the least left once the cursor leaves.
const NORMAL: Duration = Duration::from_secs(5);
const LOW: Duration = Duration::from_secs(3);
#[cfg_attr(windows, allow(dead_code))]
const RESUME: Duration = Duration::from_secs(1);
/// Cards on screen at once, and once the "N more" pill is clicked.
const STACK: usize = 3;
const EXPANDED: usize = 6;

/// Why a notification closed, as `NotificationClosed` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub(crate) enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

/// What the state asks a source to do in return: answers to the notification apps.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(windows, allow(dead_code))]
pub(crate) enum Effect {
    NotificationClosed { id: u32, reason: CloseReason },
    ActionInvoked { id: u32, key: String },
}

/// A card as it is drawn now.
pub(crate) struct CardView<'a> {
    pub(crate) notification: &'a Notification,
    pub(crate) frame: Frame,
    pub(crate) hovered: bool,
    /// Since it arrived.
    pub(crate) age: Duration,
    /// Share of its time left, from 1 to 0; `None` if it does not expire.
    pub(crate) left: Option<f32>,
}

struct Entry {
    notification: Notification,
    arrived: Instant,
    /// When it expires; `None` if it never does, while waiting, or while the cursor is over it.
    expires: Option<Instant>,
    /// Time left, kept while the cursor is over it.
    paused: Option<Duration>,
    hovered: bool,
    /// Already answered as closed: folding back.
    closing: bool,
    /// On screen (entering, shown or leaving); `None` while waiting behind the pill.
    motion: Option<Motion>,
}

impl Entry {
    fn shown(&self) -> bool {
        self.motion.is_some() && !self.closing
    }

    fn critical(&self) -> bool {
        self.notification.urgency == Urgency::Critical
    }

    /// Starts its time (when it appears or is replaced on screen).
    fn restart(&mut self, instant: Instant) {
        let lifetime = lifetime(&self.notification);
        if self.hovered {
            self.paused = lifetime;
        } else {
            self.expires = lifetime.map(|lifetime| instant + lifetime);
        }
    }
}

/// How long a notification stays: critical ones and `expire_timeout = 0` until closed.
fn lifetime(notification: &Notification) -> Option<Duration> {
    match (notification.urgency, notification.timeout) {
        (Urgency::Critical, _) | (_, Timeout::Never) => None,
        (_, Timeout::After(duration)) => Some(duration),
        (Urgency::Low, Timeout::Default) => Some(LOW),
        (Urgency::Normal, Timeout::Default) => Some(NORMAL),
    }
}

#[derive(Default)]
pub(crate) struct Notifications {
    /// In order of arrival.
    entries: Vec<Entry>,
    expanded: bool,
    effects: Vec<Effect>,
}

impl Notifications {
    /// Replaces the notification with the same `id` in place, or adds it.
    pub(super) fn receive(&mut self, notification: Notification, instant: Instant) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| !entry.closing && entry.notification.id == notification.id)
        {
            entry.notification = notification;
            if entry.motion.is_some() {
                entry.restart(instant);
            }
            return;
        }
        self.entries.push(Entry {
            notification,
            arrived: instant,
            expires: None,
            paused: None,
            hovered: false,
            closing: false,
            motion: None,
        });
    }

    /// Closes `id` with `reason`: it folds back if on screen, or leaves at once if waiting.
    pub(super) fn close(&mut self, id: u32, reason: CloseReason) {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| !entry.closing && entry.notification.id == id)
        else {
            return;
        };
        self.effects.push(Effect::NotificationClosed { id, reason });
        if self.entries[index].motion.is_some() {
            self.entries[index].closing = true;
        } else {
            self.entries.remove(index);
        }
    }

    /// Click on the card: its `default` action, if any, then closes it.
    #[cfg_attr(windows, allow(dead_code))]
    pub(super) fn click(&mut self, id: u32) {
        let default = self
            .entry(id)
            .is_some_and(|entry| entry.notification.default_action);
        self.invoke(id, default.then_some("default"));
    }

    /// Action button `key`, then closes it.
    #[cfg_attr(windows, allow(dead_code))]
    pub(super) fn act(&mut self, id: u32, key: &str) {
        self.invoke(id, Some(key));
    }

    fn invoke(&mut self, id: u32, key: Option<&str>) {
        if self.entry(id).is_none() {
            return;
        }
        if let Some(key) = key {
            self.effects.push(Effect::ActionInvoked {
                id,
                key: key.to_owned(),
            });
        }
        self.close(id, CloseReason::Dismissed);
    }

    fn entry(&self, id: u32) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.shown() && entry.notification.id == id)
    }

    /// The cursor over a card pauses its time; leaving resumes it with at least `RESUME`.
    #[cfg_attr(windows, allow(dead_code))]
    pub(super) fn hover(&mut self, id: u32, hovered: bool, instant: Instant) {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.shown() && entry.notification.id == id)
        else {
            return;
        };
        if hovered == entry.hovered {
            return;
        }
        entry.hovered = hovered;
        if hovered {
            entry.paused = entry
                .expires
                .take()
                .map(|expires| expires.saturating_duration_since(instant));
        } else if let Some(left) = entry.paused.take() {
            entry.expires = Some(instant + left.max(RESUME));
        }
    }

    /// The "N more" pill: shows up to `EXPANDED` cards.
    #[cfg_attr(windows, allow(dead_code))]
    pub(super) fn expand(&mut self) {
        self.expanded = true;
    }

    pub(super) fn expire(&mut self, instant: Instant) {
        let expired: Vec<u32> = self
            .entries
            .iter()
            .filter(|entry| entry.shown() && entry.expires.is_some_and(|at| at <= instant))
            .map(|entry| entry.notification.id)
            .collect();
        for id in expired {
            self.close(id, CloseReason::Expired);
        }
    }

    /// Folds back the closed cards, removes those already gone and brings in those waiting while there is room.
    pub(super) fn animate(&mut self, instant: Instant) {
        for entry in &mut self.entries {
            if entry.motion.is_some() {
                let wanted = !entry.closing;
                Motion::follow(&mut entry.motion, wanted, instant, instant);
            }
        }
        self.entries
            .retain(|entry| !(entry.closing && entry.motion.is_none()));

        let limit = if self.expanded { EXPANDED } else { STACK };
        let mut shown = self.entries.iter().filter(|entry| entry.shown()).count();
        // Critical ones first; otherwise in order of arrival.
        let mut waiting: Vec<usize> = (0..self.entries.len())
            .filter(|index| self.entries[*index].motion.is_none())
            .collect();
        waiting.sort_by_key(|index| !self.entries[*index].critical());
        for index in waiting {
            let entry = &mut self.entries[index];
            if shown >= limit && !entry.critical() {
                break;
            }
            Motion::follow(&mut entry.motion, true, instant, instant);
            entry.restart(instant);
            shown += 1;
        }
        if self.hidden() == 0 && shown <= STACK {
            self.expanded = false;
        }
    }

    /// The surface has just appeared (Linux): entrances start over from `instant`.
    #[cfg(target_os = "linux")]
    pub(super) fn opened(&mut self, instant: Instant) {
        for entry in &mut self.entries {
            super::restart(&mut entry.motion, instant);
        }
    }

    /// Cards on screen, top to bottom: critical ones first, then in order of arrival.
    pub(crate) fn cards(&self, frame: Instant) -> Vec<CardView<'_>> {
        let mut cards: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.motion.is_some())
            .collect();
        cards.sort_by_key(|entry| !entry.critical());
        cards
            .into_iter()
            .filter_map(|entry| {
                let motion = entry.motion?;
                let left = lifetime(&entry.notification).map(|lifetime| {
                    let left = match (entry.expires, entry.paused) {
                        (Some(expires), _) => expires.saturating_duration_since(frame),
                        (None, Some(paused)) => paused,
                        (None, None) => Duration::ZERO,
                    };
                    (left.as_secs_f32() / lifetime.as_secs_f32()).clamp(0.0, 1.0)
                });
                Some(CardView {
                    notification: &entry.notification,
                    frame: motion.frame(frame),
                    hovered: entry.hovered,
                    age: frame.saturating_duration_since(entry.arrived),
                    left,
                })
            })
            .collect()
    }

    /// Waiting behind the "N more" pill.
    pub(crate) fn hidden(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.motion.is_none())
            .count()
    }

    /// Open notifications, on screen or waiting.
    pub(crate) fn count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.closing).count()
    }

    pub(super) fn wanted(&self) -> bool {
        !self.entries.is_empty()
    }

    pub(super) fn animating(&self, frame: Instant) -> bool {
        self.entries
            .iter()
            .filter_map(|entry| entry.motion)
            .any(|motion| motion.leaving || !motion.done(frame))
    }

    /// Something will expire: the shell must keep checking the time.
    pub(super) fn momentary(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.shown() && entry.expires.is_some())
    }

    pub(super) fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32, urgency: Urgency, timeout: Timeout) -> Notification {
        Notification {
            id,
            app: "app".to_owned(),
            summary: format!("title {id}"),
            body: String::new(),
            icon: None,
            urgency,
            timeout,
            default_action: id == 1,
            actions: vec![("reply".to_owned(), "Reply".to_owned())],
            progress: None,
        }
    }

    fn normal(id: u32) -> Notification {
        note(id, Urgency::Normal, Timeout::Default)
    }

    /// Ids on screen, top to bottom (leaving ones included).
    fn shown(notifications: &Notifications, at: Instant) -> Vec<u32> {
        notifications
            .cards(at)
            .iter()
            .map(|card| card.notification.id)
            .collect()
    }

    fn closed(id: u32, reason: CloseReason) -> Effect {
        Effect::NotificationClosed { id, reason }
    }

    #[test]
    fn three_at_a_time_then_more_on_request() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        for id in 1..=5 {
            notifications.receive(normal(id), start);
        }
        notifications.animate(start);
        assert_eq!(shown(&notifications, start), [1, 2, 3]);
        assert_eq!((notifications.hidden(), notifications.count()), (2, 5));
        notifications.expand();
        notifications.animate(start);
        assert_eq!(shown(&notifications, start), [1, 2, 3, 4, 5]);
        assert_eq!(notifications.hidden(), 0);
    }

    #[test]
    fn critical_ones_go_first_even_when_full() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        for id in 1..=3 {
            notifications.receive(normal(id), start);
        }
        notifications.animate(start);
        notifications.receive(note(9, Urgency::Critical, Timeout::Default), start);
        notifications.animate(start);
        assert_eq!(shown(&notifications, start), [9, 1, 2, 3]);
        assert!(!notifications.momentary() || notifications.entries[3].expires.is_none());
    }

    #[test]
    fn expired_card_folds_back_and_the_next_enters() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        for id in 1..=4 {
            notifications.receive(normal(id), start);
        }
        notifications.animate(start);
        let later = start + Duration::from_secs(5);
        notifications.receive(note(2, Urgency::Normal, Timeout::Never), start);
        notifications.receive(note(3, Urgency::Normal, Timeout::Never), start);
        notifications.expire(later);
        assert_eq!(
            notifications.take_effects(),
            [closed(1, CloseReason::Expired)]
        );
        notifications.animate(later);
        assert_eq!(
            shown(&notifications, later),
            [1, 2, 3, 4],
            "1 leaving, 4 entering"
        );
        let gone = later + Duration::from_millis(320);
        notifications.animate(gone);
        assert_eq!(shown(&notifications, gone), [2, 3, 4]);
        assert!(notifications.wanted());
    }

    #[test]
    fn waiting_ones_do_not_expire_until_shown() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        for id in 1..=4 {
            notifications.receive(note(id, Urgency::Normal, Timeout::Never), start);
        }
        notifications.receive(normal(5), start);
        notifications.animate(start);
        notifications.expire(start + Duration::from_secs(60));
        assert!(notifications.take_effects().is_empty());
        assert_eq!(notifications.hidden(), 2);
    }

    #[test]
    fn click_action_and_dismiss_answer_the_app() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        for id in 1..=3 {
            notifications.receive(normal(id), start);
        }
        notifications.animate(start);
        notifications.click(1);
        notifications.act(2, "reply");
        notifications.close(3, CloseReason::Dismissed);
        notifications.click(2);
        let invoked = |id, key: &str| Effect::ActionInvoked {
            id,
            key: key.to_owned(),
        };
        assert_eq!(
            notifications.take_effects(),
            [
                invoked(1, "default"),
                closed(1, CloseReason::Dismissed),
                invoked(2, "reply"),
                closed(2, CloseReason::Dismissed),
                closed(3, CloseReason::Dismissed),
            ],
            "a closing card no longer answers"
        );
    }

    #[test]
    fn app_can_close_shown_or_waiting() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        for id in 1..=4 {
            notifications.receive(normal(id), start);
        }
        notifications.animate(start);
        notifications.close(4, CloseReason::Closed);
        notifications.close(1, CloseReason::Closed);
        assert_eq!(
            notifications.take_effects(),
            [
                closed(4, CloseReason::Closed),
                closed(1, CloseReason::Closed)
            ]
        );
        assert_eq!(notifications.hidden(), 0, "the waiting one leaves at once");
        notifications.animate(start);
        notifications.animate(start + Duration::from_secs(1));
        assert_eq!(
            shown(&notifications, start + Duration::from_secs(1)),
            [2, 3]
        );
    }

    #[test]
    fn replacing_keeps_the_place_and_restarts_the_time() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        notifications.receive(normal(1), start);
        notifications.receive(normal(2), start);
        notifications.animate(start);
        let later = start + Duration::from_secs(4);
        notifications.receive(normal(1), later);
        notifications.expire(start + Duration::from_secs(6));
        assert_eq!(
            notifications.take_effects(),
            [closed(2, CloseReason::Expired)]
        );
        assert_eq!(notifications.count(), 1);
    }

    #[test]
    fn hovering_pauses_and_the_bar_shows_time_left() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        notifications.receive(normal(1), start);
        notifications.animate(start);
        let half = start + Duration::from_millis(2500);
        assert!((notifications.cards(half)[0].left.unwrap() - 0.5).abs() < 0.01);
        notifications.hover(1, true, half);
        notifications.expire(start + Duration::from_secs(30));
        assert!(
            notifications.take_effects().is_empty(),
            "paused while hovered"
        );
        let late = start + Duration::from_secs(30);
        assert!((notifications.cards(late)[0].left.unwrap() - 0.5).abs() < 0.01);
        notifications.hover(1, false, late);
        notifications.expire(late + Duration::from_millis(2400));
        assert!(notifications.take_effects().is_empty());
        notifications.expire(late + Duration::from_millis(2500));
        assert_eq!(notifications.take_effects().len(), 1);
    }

    #[test]
    fn critical_and_zero_timeout_stay() {
        let start = Instant::now();
        let mut notifications = Notifications::default();
        notifications.receive(note(1, Urgency::Critical, Timeout::Default), start);
        notifications.receive(note(2, Urgency::Low, Timeout::Never), start);
        notifications.animate(start);
        assert!(!notifications.momentary());
        assert!(
            notifications
                .cards(start)
                .iter()
                .all(|card| card.left.is_none())
        );
        notifications.expire(start + Duration::from_secs(600));
        assert!(notifications.take_effects().is_empty());
    }
}
