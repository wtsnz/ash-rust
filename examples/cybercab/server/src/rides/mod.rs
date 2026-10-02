//! The Rides context, the core of the business: ride requests, the trips that serve
//! them, and the service zones demand and pricing are tracked in.

pub mod trip;
pub mod zone;

pub use trip::Trip;
pub use zone::ServiceZone;
