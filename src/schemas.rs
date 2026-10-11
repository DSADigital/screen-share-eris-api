//! Contratos JSON do serviço de screen share.
//!
//! Idênticos a `src/schemas/screen_share.rs` da Éris API (sem as anotações
//! `utoipa::ToSchema`, usadas apenas para gerar o Swagger em produção).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScreenShareResponse {
    pub session_id: Uuid,
    pub code: String,
    pub ticket: String,
    pub websocket_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinScreenShareRequest {
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinScreenShareResponse {
    pub session_id: Uuid,
    pub code: String,
    pub ticket: String,
    pub websocket_url: String,
    pub status: String,
    pub current_broadcaster: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantDto {
    pub user_id: Uuid,
    pub username: String,
    pub first_name: String,
    pub is_broadcasting: bool,
    pub is_connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenShareStateDto {
    pub session_id: Uuid,
    pub code: String,
    pub status: String,
    pub participants: Vec<ParticipantDto>,
    pub broadcaster_id: Option<Uuid>,
    pub is_swapping: bool,
}

/// Mensagens enviadas do cliente para o servidor via WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "payload")]
pub enum ClientSignalMessage {
    #[serde(rename = "join")]
    Join { ticket: String },

    #[serde(rename = "start_broadcast")]
    StartBroadcast,

    #[serde(rename = "stop_broadcast")]
    StopBroadcast,

    #[serde(rename = "request_swap")]
    RequestSwap,

    #[serde(rename = "respond_swap")]
    RespondSwap { accept: bool },

    #[serde(rename = "sdp_offer")]
    SdpOffer { sdp: String },

    #[serde(rename = "sdp_answer")]
    SdpAnswer { sdp: String },

    #[serde(rename = "ice_candidate")]
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
    },

    #[serde(rename = "ping")]
    Ping,
}

/// Mensagens enviadas do servidor para o cliente via WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "payload")]
pub enum ServerSignalMessage {
    #[serde(rename = "authenticated")]
    Authenticated { user_id: Uuid, session_id: Uuid },

    #[serde(rename = "session_state")]
    SessionState(ScreenShareStateDto),

    #[serde(rename = "participant_joined")]
    ParticipantJoined {
        user_id: Uuid,
        username: String,
        first_name: String,
    },

    #[serde(rename = "participant_left")]
    ParticipantLeft { user_id: Uuid },

    #[serde(rename = "broadcast_started")]
    BroadcastStarted {
        broadcaster_id: Uuid,
        username: String,
    },

    #[serde(rename = "broadcast_stopped")]
    BroadcastStopped { reason: String },

    #[serde(rename = "swap_requested")]
    SwapRequested {
        requester_id: Uuid,
        requester_name: String,
    },

    #[serde(rename = "swap_response")]
    SwapResponse {
        accepted: bool,
        new_broadcaster_id: Option<Uuid>,
    },

    #[serde(rename = "sdp_offer")]
    SdpOffer { sdp: String, from_user_id: Uuid },

    #[serde(rename = "sdp_answer")]
    SdpAnswer { sdp: String, from_user_id: Uuid },

    #[serde(rename = "ice_candidate")]
    IceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_m_line_index: Option<u32>,
        from_user_id: Uuid,
    },

    #[serde(rename = "session_closed")]
    SessionClosed { reason: String },

    #[serde(rename = "error")]
    Error { code: String, message: String },

    #[serde(rename = "pong")]
    Pong,
}

// ---------------------------------------------------------------------------
// Exclusivo desta demo: identidade efêmera (substitui login/JWT da Éris API)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIdentityRequest {
    pub first_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIdentityResponse {
    pub token: String,
    pub user_id: Uuid,
    pub username: String,
    pub first_name: String,
}
