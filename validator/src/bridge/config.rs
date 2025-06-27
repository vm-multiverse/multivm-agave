//! Configuration module for bridge settings
//!
//! This module provides centralized configuration for Solana RPC and WebSocket URLs.
//!
//! **IMPORTANT**: The default URLs are configured for internal network tunneling.
//! You MUST manually change these addresses before execution to match your actual
//! Solana node endpoints.

/// Default Solana node configuration
///
/// **WARNING**: These are internal network tunnel addresses and must be changed
/// before use in production or different network environments.
pub struct MultivmConfig;

impl MultivmConfig {
    /// Default RPC URL for Solana node
    /// **NOTE**: This is an internal network tunnel address - change before use!
    pub const RPC_URL: &'static str = "http://100.68.83.77:8899";
    
    /// Default WebSocket URL for Solana node
    /// **NOTE**: This is an internal network tunnel address - change before use!
    pub const WEBSOCKET_URL: &'static str = "ws://100.68.83.77:8900";
    
    /// Get the default RPC URL
    pub fn rpc_url() -> String {
        Self::RPC_URL.to_string()
    }
    
    /// Get the default WebSocket URL
    pub fn websocket_url() -> String {
        Self::WEBSOCKET_URL.to_string()
    }
    
    /// Get both URLs as a tuple (rpc_url, websocket_url)
    pub fn urls() -> (String, String) {
        (Self::rpc_url(), Self::websocket_url())
    }
}