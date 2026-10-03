// Local identity (plan/12): owner bootstrap and token rotation, agent
// credentials, browser sessions. Split by responsibility; the row plumbing
// shared across the seams lives in `credentials`.
mod agents;
mod bootstrap;
mod credentials;
mod sessions;

#[cfg(test)]
mod tests;

pub use bootstrap::IdentityPaths;
