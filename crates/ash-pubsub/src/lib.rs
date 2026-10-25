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
use std::sync::{Arc, RwLock, Weak};
use tokio::sync::broadcast;

use ash_core::{CommittedEvent, Context, EventReader, Notification, Notifier, ResourceDef, Result};

pub const DEFAULT_CAPACITY: usize = 8192;

/// Tenant and topic pattern are separate routing components. Wildcards cannot match
/// another tenant, including tenant names containing delimiters or wildcards.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TopicKey {
    pub tenant: Option<String>,
    pub pattern: String,
}

#[derive(Clone, Debug)]
pub struct PubSub {
    inner: Arc<RwLock<PubSubInner>>,
}
#[derive(Debug)]
struct Channel {
    key: TopicKey,
    reader: EventReader,
    sender: broadcast::Sender<Arc<Notification>>,
}
#[derive(Debug)]
struct PubSubInner {
    channels: HashMap<u64, Channel>,
    next_id: u64,
    capacity: usize,
}
impl Default for PubSub {
    fn default() -> Self {
        Self::new()
    }
}
impl PubSub {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(PubSubInner {
                channels: HashMap::new(),
                next_id: 0,
                capacity,
            })),
        }
    }
    /// Register a scope-bound reader. Unsupported snapshot dependencies fail here.
    pub fn subscribe<D>(
        &self,
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        pattern: impl Into<String>,
    ) -> Result<Subscription> {
        self.subscribe_reader(ctx.event_reader(resource, None)?, pattern)
    }
    pub fn subscribe_reader(
        &self,
        reader: EventReader,
        pattern: impl Into<String>,
    ) -> Result<Subscription> {
        let key = TopicKey {
            tenant: reader.tenant().map(str::to_owned),
            pattern: pattern.into(),
        };
        let mut inner = self.inner.write().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;
        let (sender, receiver) = broadcast::channel(inner.capacity);
        inner.channels.insert(
            id,
            Channel {
                key: key.clone(),
                reader,
                sender,
            },
        );
        Ok(Subscription {
            key,
            id,
            inner: Arc::downgrade(&self.inner),
            receiver,
        })
    }
    /// Only core-created opaque events can be routed. Every receiver is independently
    /// admitted and projected before anything enters its queue.
    /// Returns the number of subscriber queues that accepted the event.
    pub fn publish_topics(
        &self,
        topics: &[impl AsRef<str>],
        event: &CommittedEvent,
    ) -> Result<usize> {
        self.publish_with(event, |_| {
            topics
                .iter()
                .map(|topic| topic.as_ref().to_owned())
                .collect()
        })
    }
    fn publish_with(
        &self,
        event: &CommittedEvent,
        topics: impl Fn(&Notification) -> Vec<String>,
    ) -> Result<usize> {
        let inner = self.inner.read().unwrap();
        let mut delivered = 0;
        for channel in inner.channels.values() {
            if let Some(notification) = event.project(&channel.reader)?
                && channel.key.tenant == notification.tenant
                && topics(&notification)
                    .iter()
                    .any(|topic| topic_matches(&channel.key.pattern, topic))
                && channel.sender.send(Arc::new(notification)).is_ok()
            {
                delivered += 1;
            }
        }
        Ok(delivered)
    }
    pub fn publish(&self, topic: &str, event: &CommittedEvent) -> Result<usize> {
        self.publish_topics(&[topic], event)
    }
    pub fn subscribe_all<R: ash_core::Resource>(
        &self,
        ctx: &Context<impl Sized>,
    ) -> Result<Subscription> {
        self.subscribe(ctx, &R::DEF, format!("{}:*", R::DEF.name.to_lowercase()))
    }
    pub fn subscribe_resource<R: ash_core::Resource>(
        &self,
        ctx: &Context<impl Sized>,
        action: &str,
    ) -> Result<Subscription> {
        self.subscribe(
            ctx,
            &R::DEF,
            format!("{}:{action}", R::DEF.name.to_lowercase()),
        )
    }
    pub fn subscribe_record<R: ash_core::Resource>(
        &self,
        ctx: &Context<impl Sized>,
        id: impl Into<ash_core::Value>,
        action: Option<&str>,
    ) -> Result<Subscription> {
        self.subscribe(
            ctx,
            &R::DEF,
            format!(
                "{}:{}:{}",
                R::DEF.name.to_lowercase(),
                id.into(),
                action.unwrap_or("*")
            ),
        )
    }
}

pub struct Subscription {
    key: TopicKey,
    id: u64,
    inner: Weak<RwLock<PubSubInner>>,
    receiver: broadcast::Receiver<Arc<Notification>>,
}
impl Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("key", &self.key)
            .finish()
    }
}
impl Subscription {
    pub fn pattern(&self) -> &str {
        &self.key.pattern
    }
    pub fn topic_key(&self) -> &TopicKey {
        &self.key
    }
    pub async fn recv(
        &mut self,
    ) -> std::result::Result<Arc<Notification>, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.write().unwrap().channels.remove(&self.id);
        }
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
    fn notify_event<'a>(
        &'a self,
        event: &'a CommittedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let result = self.pubsub.publish_with(event, |notification| {
            if let Some(formatter) = &self.topics_fn {
                formatter(notification)
            } else {
                let res = notification.resource.to_lowercase();
                let act = &notification.action;
                let id = &notification.id;
                vec![format!("{res}:{act}"), format!("{res}:{id}:{act}")]
            }
        });
        Box::pin(std::future::ready(result.map(|_| ())))
    }
    fn notify<'a>(
        &'a self,
        _notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(std::future::ready(Err(ash_core::Error::Invalid(
            "PubSub requires a core committed event".into(),
        ))))
    }
}

/// Extension trait for [`ash_core::Context`] providing fluent PubSub attachment.
pub trait ContextPubSubExt<D> {
    /// Attach a [`PubSub`] broker directly to the context using a [`PubSubNotifier`].
    fn with_pubsub(self, pubsub: Arc<PubSub>) -> Self;
}

// Runtime notifiers are inherited by every context it binds.
impl<D> ContextPubSubExt<D> for ash_core::Runtime<D> {
    fn with_pubsub(self, pubsub: Arc<PubSub>) -> Self {
        self.with_notifier(Arc::new(PubSubNotifier::new(pubsub)))
    }
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
    fn subscribe_all(pubsub: &PubSub, ctx: &Context<impl Sized>) -> Result<Subscription> {
        pubsub.subscribe_all::<Self>(ctx)
    }

    /// Subscribe to a specific action event for this resource type (`"<resource>:<action>"`).
    fn subscribe_action(
        pubsub: &PubSub,
        ctx: &Context<impl Sized>,
        action: &str,
    ) -> Result<Subscription> {
        pubsub.subscribe_resource::<Self>(ctx, action)
    }

    /// Subscribe to events for this specific record instance.
    ///
    /// If `action` is `Some("pay")`, subscribes to `"<resource>:<id>:pay"`.
    /// If `action` is `None`, subscribes to `"<resource>:<id>:*"`.
    fn subscribe(
        &self,
        pubsub: &PubSub,
        ctx: &Context<impl Sized>,
        action: Option<&str>,
    ) -> Result<Subscription> {
        pubsub.subscribe_record::<Self>(ctx, self.pk(), action)
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
