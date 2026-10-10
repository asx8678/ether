// Modified from OpenAI Codex (codex-rs/protocol/src/network_policy.rs @ 806d973): network-proxy types are local copies.
use crate::approvals::NetworkApprovalProtocol;
use crate::network_proxy_types::NetworkDecisionSource;
use crate::network_proxy_types::NetworkPolicyDecision;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyDecisionPayload {
    pub decision: NetworkPolicyDecision,
    pub source: NetworkDecisionSource,
    #[serde(default)]
    pub protocol: Option<NetworkApprovalProtocol>,
    pub host: Option<String>,
    pub reason: Option<String>,
    pub port: Option<u16>,
}

impl NetworkPolicyDecisionPayload {
    pub fn is_ask_from_decider(&self) -> bool {
        self.decision == NetworkPolicyDecision::Ask && self.source == NetworkDecisionSource::Decider
    }
}
