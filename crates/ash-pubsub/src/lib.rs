//! # Ash PubSub
//!
//! Pattern-based, in-memory PubSub event broker and action notifier for `ash-rust`.
//!
//! Mirrors Elixir's `ash_pubsub` extension: resources and actions emit lifecycle
//! notifications via `ash-core::Notifier`, and `ash-pubsub` broadcasts them to
//! pattern-matched subscription streams with deduplicated multi-topic delivery.

use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

use ash_core::{Notification, Notifier, Result};
use uuid::Uuid;

/// In-memory, pattern-based PubSub event broker for Ash resources.
#[derive(Clone, Debug)]
pub struct PubSub {
    inner: Arc<RwLock<PubSubInner>>,
}

#[derive(Debug)]
struct PubSubInner {
    channels: HashMap<String, broadcast::Sender<Notification>>,
    capacity: usize,
}

impl Default for PubSub {
    fn default() -> Self {
        Self::new()
    }
}

impl PubSub {
    /// Create a new PubSub broker with default buffer capacity of 256.
    pub fn new() -> Self {
        Self::with_capacity(256)
    }

    /// Create a new PubSub broker with a custom buffer capacity per topic channel.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(PubSubInner {
                channels: HashMap::new(),
                capacity,
            })),
        }
    }

    /// Subscribe to a topic pattern (e.g. `"orders:*"`, `"order:create"`, `"*"`, `"orders:*:paid"`).
    pub fn subscribe(&self, pattern: impl Into<String>) -> Subscription {
        let pattern_str = pattern.into();
        let mut inner = self.inner.write().unwrap();
        let capacity = inner.capacity;
        let sender = inner
            .channels
            .entry(pattern_str.clone())
            .or_insert_with(|| broadcast::channel(capacity).0);

        Subscription {
            pattern: pattern_str,
            receiver: sender.subscribe(),
        }
    }

    /// Publish a notification to multiple topics simultaneously, ensuring each matching
    /// subscriber receives the notification exactly once per event.
    pub fn publish_topics(&self, topics: &[impl AsRef<str>], notification: Notification) -> usize {
        let inner = self.inner.read().unwrap();
        let mut delivered = 0;

        for (pattern, sender) in &inner.channels {
            let matched = topics.iter().any(|t| topic_matches(pattern, t.as_ref()));
            if matched && sender.send(notification.clone()).is_ok() {
                delivered += 1;
            }
        }

        delivered
    }

    /// Publish a notification to a specific concrete topic (e.g. `"order:create"`).
    ///
    /// The notification will be broadcast to all subscribers whose pattern matches `topic`.
    /// Returns the number of distinct patterns to which the notification was delivered.
    pub fn publish(&self, topic: &str, notification: Notification) -> usize {
        self.publish_topics(&[topic], notification)
    }

    /// Subscribe to all lifecycle events for a specific resource type (e.g. `"order:*"`).
    pub fn subscribe_all<R: ash_core::Resource>(&self) -> Subscription {
        let prefix = R::DEF.name.to_lowercase();
        self.subscribe(format!("{prefix}:*"))
    }

    /// Subscribe to a specific action event for a resource type (e.g. `"order:create"`).
    pub fn subscribe_resource<R: ash_core::Resource>(&self, action: &str) -> Subscription {
        let prefix = R::DEF.name.to_lowercase();
        self.subscribe(format!("{prefix}:{action}"))
    }

    /// Subscribe to events for a specific record ID (e.g. `"order:<id>:*"` or `"order:<id>:action"`).
    pub fn subscribe_record<R: ash_core::Resource>(
        &self,
        id: Uuid,
        action: Option<&str>,
    ) -> Subscription {
        let prefix = R::DEF.name.to_lowercase();
        match action {
            Some(act) => self.subscribe(format!("{prefix}:{id}:{act}")),
            None => self.subscribe(format!("{prefix}:{id}:*")),
        }
    }
}

/// Active subscription stream receiving notifications matching a pattern.
pub struct Subscription {
    pattern: String,
    receiver: broadcast::Receiver<Notification>,
}

impl Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("pattern", &self.pattern)
            .finish()
    }
}

impl Subscription {
    /// Get the pattern this subscription was created with.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Asynchronously wait for the next notification matching this subscription's pattern.
    pub async fn recv(&mut self) -> std::result::Result<Notification, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

/// Check if a topic matches a subscription pattern.
///
/// Rules:
/// - `"*"` matches all topics.
/// - Segments separated by `:` (e.g. `"order:create"`):
///   - Exact segment name must match.
///   - `"*"` matches any single segment.
///   - Trailing `"*"` matches all remaining segments (e.g. `"orders:*"` matches `"orders:create"`
///     and `"orders:123:updated"`).
pub fn topic_matches(pattern: &str, topic: &str) -> bool {
    if pattern == "*" || pattern == topic {
        return true;
    }

    let p_segs: Vec<&str> = pattern.split(':').collect();
    let t_segs: Vec<&str> = topic.split(':').collect();

    // Trailing wildcard matches remainder of topic
    if p_segs.last() == Some(&"*") {
        let prefix_len = p_segs.len() - 1;
        if t_segs.len() < prefix_len {
            return false;
        }
        for i in 0..prefix_len {
            if p_segs[i] != "*" && p_segs[i] != t_segs[i] {
                return false;
            }
        }
        return true;
    }

    if p_segs.len() != t_segs.len() {
        return false;
    }

    for (p, t) in p_segs.iter().zip(t_segs.iter()) {
        if *p != "*" && p != t {
            return false;
        }
    }

    true
}

/// Type alias for custom topic formatting functions.
pub type TopicFormatter = Arc<dyn Fn(&Notification) -> Vec<String> + Send + Sync>;

/// A [`Notifier`] that broadcasts notifications to a [`PubSub`] broker.
pub struct PubSubNotifier {
    pubsub: Arc<PubSub>,
    topics_fn: Option<TopicFormatter>,
}

impl Debug for PubSubNotifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PubSubNotifier")
            .field("pubsub", &self.pubsub)
            .finish()
    }
}

impl PubSubNotifier {
    /// Create a new PubSubNotifier backed by the given PubSub broker.
    ///
    /// By default, broadcasts to two topics for each event:
    /// - `<resource_lowercase>:<action>` (e.g. `"order:create"`)
    /// - `<resource_lowercase>:<id>:<action>` (e.g. `"order:<uuid>:pay"`)
    pub fn new(pubsub: Arc<PubSub>) -> Self {
        Self {
            pubsub,
            topics_fn: None,
        }
    }

    /// Configure a custom closure mapping a [`Notification`] to a list of topic strings.
    pub fn with_topics<F>(mut self, f: F) -> Self
    where
        F: Fn(&Notification) -> Vec<String> + Send + Sync + 'static,
    {
        self.topics_fn = Some(Arc::new(f));
        self
    }
}

impl Notifier for PubSubNotifier {
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let topics = if let Some(ref f) = self.topics_fn {
            f(notification)
        } else {
            let res = notification.resource.to_lowercase();
            let act = &notification.action;
            let id = notification.id;
            vec![format!("{res}:{act}"), format!("{res}:{id}:{act}")]
        };

        self.pubsub.publish_topics(&topics, notification.clone());

        Box::pin(std::future::ready(Ok(())))
    }
}

/// Extension trait for [`ash_core::Context`] providing fluent PubSub attachment.
pub trait ContextPubSubExt<D> {
    /// Attach a [`PubSub`] broker directly to the context using a [`PubSubNotifier`].
    fn with_pubsub(self, pubsub: Arc<PubSub>) -> Self;
}

impl<D> ContextPubSubExt<D> for ash_core::Context<D> {
    fn with_pubsub(self, pubsub: Arc<PubSub>) -> Self {
        let notifier = Arc::new(PubSubNotifier::new(pubsub));
        self.with_notifier(notifier)
    }
}

/// Extension trait implemented for all Ash resources providing typed subscription helpers.
pub trait PubSubResourceExt: ash_core::Resource {
    /// Subscribe to all lifecycle events for this resource type (`"<resource>:*"`).
    fn subscribe_all(pubsub: &PubSub) -> Subscription {
        pubsub.subscribe_all::<Self>()
    }

    /// Subscribe to a specific action event for this resource type (`"<resource>:<action>"`).
    fn subscribe_action(pubsub: &PubSub, action: &str) -> Subscription {
        pubsub.subscribe_resource::<Self>(action)
    }

    /// Subscribe to events for this specific record instance.
    ///
    /// If `action` is `Some("pay")`, subscribes to `"<resource>:<id>:pay"`.
    /// If `action` is `None`, subscribes to `"<resource>:<id>:*"`.
    fn subscribe(&self, pubsub: &PubSub, action: Option<&str>) -> Subscription {
        pubsub.subscribe_record::<Self>(self.id(), action)
    }
}

impl<R: ash_core::Resource> PubSubResourceExt for R {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_pattern_matching() {
        assert!(topic_matches("*", "anything"));
        assert!(topic_matches("orders:created", "orders:created"));
        assert!(!topic_matches("orders:created", "orders:paid"));

        // Single wildcard segment
        assert!(topic_matches("orders:*:paid", "orders:123:paid"));
        assert!(!topic_matches("orders:*:paid", "orders:123:cancelled"));

        // Trailing wildcard
        assert!(topic_matches("orders:*", "orders:created"));
        assert!(topic_matches("orders:*", "orders:123:paid"));
        assert!(!topic_matches("orders:*", "users:created"));
    }
}
