//! Máquina de estados das sessões de screen share.
//!
//! Este arquivo é uma CÓPIA FIEL de `src/services/screen_share/manager.rs`
//! da Éris API. A única diferença é o caminho de import dos schemas.
//!
//! Observe que todo o estado vive apenas em memória (`HashMap`s protegidos
//! por `RwLock`). Nada aqui recebe, armazena ou inspeciona pixels de tela:
//! o servidor apenas repassa mensagens de sinalização WebRTC (SDP/ICE) para
//! o outro participante da mesma sessão.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::schemas::{
    ParticipantDto, ScreenShareStateDto, ServerSignalMessage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    WaitingForPeer,
    Ready,
    Closed,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::WaitingForPeer => "waiting_for_peer",
            SessionStatus::Ready => "ready",
            SessionStatus::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Participant {
    pub user_id: Uuid,
    pub username: String,
    pub first_name: String,
    pub is_connected: bool,
    pub tx: Option<mpsc::UnboundedSender<ServerSignalMessage>>,
}

#[derive(Debug, Clone)]
pub struct SwapRequest {
    pub requester_id: Uuid,
    #[allow(dead_code)]
    pub requested_at: Instant,
}

#[derive(Debug, Clone)]
pub struct TicketData {
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub expires_at: Instant,
}

pub struct ScreenShareSession {
    pub id: Uuid,
    pub code: String,
    #[allow(dead_code)]
    pub created_by: Uuid,
    pub status: SessionStatus,
    pub participants: HashMap<Uuid, Participant>,
    pub broadcaster_id: Option<Uuid>,
    pub swap_request: Option<SwapRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenShareError {
    SessionNotFound,
    SessionClosed,
    SessionFull,
    UserNotFound,
    Unauthorized,
    AlreadyBroadcasting,
    AnotherUserBroadcasting,
    NoActiveBroadcast,
    CannotSwapWithSelf,
    SwapAlreadyPending,
    NoPendingSwap,
    InvalidTicket,
    PeerNotConnected,
}

impl std::fmt::Display for ScreenShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScreenShareError::SessionNotFound => write!(f, "Sessão não encontrada."),
            ScreenShareError::SessionClosed => write!(f, "Sessão já foi encerrada."),
            ScreenShareError::SessionFull => {
                write!(f, "Sessão cheia. Limite de 2 participantes atingido.")
            }
            ScreenShareError::UserNotFound => write!(f, "Usuário não pertence à sessão."),
            ScreenShareError::Unauthorized => write!(f, "Ação não autorizada."),
            ScreenShareError::AlreadyBroadcasting => {
                write!(f, "Você já está transmitindo sua tela.")
            }
            ScreenShareError::AnotherUserBroadcasting => {
                write!(f, "Outro participante está transmitindo. Solicite a troca.")
            }
            ScreenShareError::NoActiveBroadcast => {
                write!(f, "Nenhuma transmissão ativa no momento.")
            }
            ScreenShareError::CannotSwapWithSelf => {
                write!(f, "Você não pode solicitar troca para si mesmo.")
            }
            ScreenShareError::SwapAlreadyPending => {
                write!(f, "Já existe uma solicitação de troca em andamento.")
            }
            ScreenShareError::NoPendingSwap => write!(f, "Nenhuma solicitação de troca pendente."),
            ScreenShareError::InvalidTicket => write!(f, "Ticket de conexão inválido ou expirado."),
            ScreenShareError::PeerNotConnected => {
                write!(f, "O outro participante não está conectado via WebSocket.")
            }
        }
    }
}

impl std::error::Error for ScreenShareError {}

#[derive(Clone)]
pub struct ScreenShareManager {
    sessions: Arc<RwLock<HashMap<Uuid, ScreenShareSession>>>,
    code_to_id: Arc<RwLock<HashMap<String, Uuid>>>,
    tickets: Arc<RwLock<HashMap<String, TicketData>>>,
}

impl ScreenShareManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            code_to_id: Arc::new(RwLock::new(HashMap::new())),
            tickets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Cria uma nova sessão com o criador (Participante A)
    pub async fn create_session(
        &self,
        session_id: Uuid,
        code: String,
        creator_id: Uuid,
        creator_username: String,
        creator_first_name: String,
    ) -> Result<String, ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let mut code_map = self.code_to_id.write().await;

        let mut participants = HashMap::new();
        participants.insert(
            creator_id,
            Participant {
                user_id: creator_id,
                username: creator_username,
                first_name: creator_first_name,
                is_connected: false,
                tx: None,
            },
        );

        let session = ScreenShareSession {
            id: session_id,
            code: code.clone(),
            created_by: creator_id,
            status: SessionStatus::WaitingForPeer,
            participants,
            broadcaster_id: None,
            swap_request: None,
        };

        sessions.insert(session_id, session);
        code_map.insert(code, session_id);

        let ticket = self.issue_ticket(session_id, creator_id).await;
        Ok(ticket)
    }

    /// Emite um ticket de conexão temporário (válido por 60 segundos)
    pub async fn issue_ticket(&self, session_id: Uuid, user_id: Uuid) -> String {
        let ticket = nanoid::nanoid!(32);
        let mut tickets = self.tickets.write().await;
        tickets.insert(
            ticket.clone(),
            TicketData {
                session_id,
                user_id,
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );
        ticket
    }

    /// Valida e consome um ticket
    pub async fn consume_ticket(&self, ticket: &str) -> Result<(Uuid, Uuid), ScreenShareError> {
        let mut tickets = self.tickets.write().await;
        if let Some(data) = tickets.remove(ticket) {
            if Instant::now() <= data.expires_at {
                return Ok((data.session_id, data.user_id));
            }
        }
        Err(ScreenShareError::InvalidTicket)
    }

    /// Resolve o ID da sessão a partir de um código
    pub async fn resolve_code(&self, code: &str) -> Option<Uuid> {
        let code_map = self.code_to_id.read().await;
        code_map.get(code).copied()
    }

    /// Entrada de um participante na sessão (máximo 2 simultâneos)
    pub async fn join_session(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        username: String,
        first_name: String,
    ) -> Result<String, ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        if !session.participants.contains_key(&user_id) {
            if session.participants.len() >= 2 {
                return Err(ScreenShareError::SessionFull);
            }

            session.participants.insert(
                user_id,
                Participant {
                    user_id,
                    username,
                    first_name,
                    is_connected: false,
                    tx: None,
                },
            );

            if session.participants.len() == 2 {
                session.status = SessionStatus::Ready;
            }
        }

        let ticket = self.issue_ticket(session_id, user_id).await;
        Ok(ticket)
    }

    /// Registra a conexão WebSocket ativa do participante
    pub async fn attach_ws(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        tx: mpsc::UnboundedSender<ServerSignalMessage>,
    ) -> Result<ScreenShareStateDto, ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        let (joining_username, joining_first_name) = {
            let participant = session
                .participants
                .get_mut(&user_id)
                .ok_or(ScreenShareError::UserNotFound)?;
            participant.is_connected = true;
            participant.tx = Some(tx);
            (participant.username.clone(), participant.first_name.clone())
        };

        let state_dto = self.build_state_dto(session);

        // Notifica o outro participante sobre a conexão deste e atualiza o estado
        for (pid, p) in &session.participants {
            if *pid != user_id {
                if let Some(ref peer_tx) = p.tx {
                    let _ = peer_tx.send(ServerSignalMessage::ParticipantJoined {
                        user_id,
                        username: joining_username.clone(),
                        first_name: joining_first_name.clone(),
                    });
                    let _ = peer_tx.send(ServerSignalMessage::SessionState(state_dto.clone()));
                }
            }
        }

        Ok(state_dto)
    }

    /// Desconexão do WebSocket de um participante
    pub async fn detach_ws(
        &self,
        session_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        let mut stopped_broadcast = false;
        if let Some(participant) = session.participants.get_mut(&user_id) {
            participant.is_connected = false;
            participant.tx = None;
        }

        // Se o participante que desconectou era o transmissor ativo, cessa a transmissão
        if session.broadcaster_id == Some(user_id) {
            session.broadcaster_id = None;
            stopped_broadcast = true;
        }

        // Cancela solicitação de troca se envolver o participante desconectado
        if let Some(ref swap) = session.swap_request {
            if swap.requester_id == user_id || session.broadcaster_id == Some(user_id) {
                session.swap_request = None;
            }
        }

        // Notifica o participante remanescente
        for (pid, p) in &session.participants {
            if *pid != user_id {
                if let Some(ref peer_tx) = p.tx {
                    let _ = peer_tx.send(ServerSignalMessage::ParticipantLeft { user_id });
                    if stopped_broadcast {
                        let _ = peer_tx.send(ServerSignalMessage::BroadcastStopped {
                            reason: "O transmissor desconectou-se da sessão.".to_string(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Inicia transmissão de tela
    pub async fn start_broadcast(
        &self,
        session_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        if !session.participants.contains_key(&user_id) {
            return Err(ScreenShareError::UserNotFound);
        }

        if let Some(active_id) = session.broadcaster_id {
            if active_id == user_id {
                return Err(ScreenShareError::AlreadyBroadcasting);
            } else {
                return Err(ScreenShareError::AnotherUserBroadcasting);
            }
        }

        session.broadcaster_id = Some(user_id);
        let broadcaster_username = session
            .participants
            .get(&user_id)
            .map(|p| p.username.clone())
            .unwrap_or_default();

        // Notifica ambos os participantes
        for p in session.participants.values() {
            if let Some(ref tx) = p.tx {
                let _ = tx.send(ServerSignalMessage::BroadcastStarted {
                    broadcaster_id: user_id,
                    username: broadcaster_username.clone(),
                });
            }
        }

        Ok(())
    }

    /// Encerra transmissão de tela ativa sem fechar a sessão
    pub async fn stop_broadcast(
        &self,
        session_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        if session.broadcaster_id != Some(user_id) {
            return Err(ScreenShareError::Unauthorized);
        }

        session.broadcaster_id = None;
        session.swap_request = None;

        // Notifica ambos os participantes
        for p in session.participants.values() {
            if let Some(ref tx) = p.tx {
                let _ = tx.send(ServerSignalMessage::BroadcastStopped {
                    reason: "Transmissão encerrada pelo usuário.".to_string(),
                });
            }
        }

        Ok(())
    }

    /// Solicita troca de transmissor (participante B pede para assumir transmissão de A)
    pub async fn request_swap(
        &self,
        session_id: Uuid,
        requester_id: Uuid,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        if !session.participants.contains_key(&requester_id) {
            return Err(ScreenShareError::UserNotFound);
        }

        let current_broadcaster = session
            .broadcaster_id
            .ok_or(ScreenShareError::NoActiveBroadcast)?;

        if current_broadcaster == requester_id {
            return Err(ScreenShareError::CannotSwapWithSelf);
        }

        if session.swap_request.is_some() {
            return Err(ScreenShareError::SwapAlreadyPending);
        }

        let requester_name = session
            .participants
            .get(&requester_id)
            .map(|p| p.first_name.clone())
            .unwrap_or_default();

        session.swap_request = Some(SwapRequest {
            requester_id,
            requested_at: Instant::now(),
        });

        // Notifica o transmissor ativo sobre o pedido de troca
        if let Some(broadcaster) = session.participants.get(&current_broadcaster) {
            if let Some(ref tx) = broadcaster.tx {
                let _ = tx.send(ServerSignalMessage::SwapRequested {
                    requester_id,
                    requester_name,
                });
            }
        }

        Ok(())
    }

    /// Responde a solicitação de troca (aceita ou recusa)
    pub async fn respond_swap(
        &self,
        session_id: Uuid,
        user_id: Uuid,
        accept: bool,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        if session.broadcaster_id != Some(user_id) {
            return Err(ScreenShareError::Unauthorized);
        }

        let swap = session
            .swap_request
            .take()
            .ok_or(ScreenShareError::NoPendingSwap)?;

        let requester_id = swap.requester_id;

        if !accept {
            // Notifica o solicitante da recusa
            if let Some(requester) = session.participants.get(&requester_id) {
                if let Some(ref tx) = requester.tx {
                    let _ = tx.send(ServerSignalMessage::SwapResponse {
                        accepted: false,
                        new_broadcaster_id: None,
                    });
                }
            }
            return Ok(());
        }

        // Troca aceita: handover coordenado
        session.broadcaster_id = Some(requester_id);
        let new_broadcaster_username = session
            .participants
            .get(&requester_id)
            .map(|p| p.username.clone())
            .unwrap_or_default();

        // Notifica ambos
        for p in session.participants.values() {
            if let Some(ref tx) = p.tx {
                let _ = tx.send(ServerSignalMessage::BroadcastStopped {
                    reason: "Alternância de transmissão realizada.".to_string(),
                });
                let _ = tx.send(ServerSignalMessage::SwapResponse {
                    accepted: true,
                    new_broadcaster_id: Some(requester_id),
                });
                let _ = tx.send(ServerSignalMessage::BroadcastStarted {
                    broadcaster_id: requester_id,
                    username: new_broadcaster_username.clone(),
                });
            }
        }

        Ok(())
    }

    /// Encaminha mensagens de sinalização (SDP Offer/Answer/ICE) exclusivamente ao peer
    pub async fn route_signaling(
        &self,
        session_id: Uuid,
        from_user_id: Uuid,
        message: ServerSignalMessage,
    ) -> Result<(), ScreenShareError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.status == SessionStatus::Closed {
            return Err(ScreenShareError::SessionClosed);
        }

        let peer = session
            .participants
            .iter()
            .find(|(pid, _)| **pid != from_user_id)
            .map(|(_, p)| p)
            .ok_or(ScreenShareError::PeerNotConnected)?;

        if let Some(ref tx) = peer.tx {
            let _ = tx.send(message);
            Ok(())
        } else {
            Err(ScreenShareError::PeerNotConnected)
        }
    }

    /// Encerramento definitivo da sessão e liberação de recursos
    pub async fn close_session(
        &self,
        session_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if !session.participants.contains_key(&user_id) {
            return Err(ScreenShareError::Unauthorized);
        }

        session.status = SessionStatus::Closed;
        session.broadcaster_id = None;
        session.swap_request = None;

        // Notifica e desconecta todos os participantes
        for p in session.participants.values() {
            if let Some(ref tx) = p.tx {
                let _ = tx.send(ServerSignalMessage::SessionClosed {
                    reason: "Sessão encerrada por um dos participantes.".to_string(),
                });
            }
        }

        let code = session.code.clone();
        drop(sessions);

        let mut code_map = self.code_to_id.write().await;
        code_map.remove(&code);

        Ok(())
    }

    /// Saída voluntária de um participante
    pub async fn leave_session(
        &self,
        session_id: Uuid,
        user_id: Uuid,
    ) -> Result<(), ScreenShareError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        if session.participants.remove(&user_id).is_some() {
            if session.broadcaster_id == Some(user_id) {
                session.broadcaster_id = None;
            }
            if let Some(ref swap) = session.swap_request {
                if swap.requester_id == user_id {
                    session.swap_request = None;
                }
            }

            session.status = SessionStatus::WaitingForPeer;

            for p in session.participants.values() {
                if let Some(ref tx) = p.tx {
                    let _ = tx.send(ServerSignalMessage::ParticipantLeft { user_id });
                }
            }
        }

        Ok(())
    }

    /// Consulta estado atual da sessão
    pub async fn get_session_state(
        &self,
        session_id: Uuid,
    ) -> Result<ScreenShareStateDto, ScreenShareError> {
        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&session_id)
            .ok_or(ScreenShareError::SessionNotFound)?;

        Ok(self.build_state_dto(session))
    }

    fn build_state_dto(&self, session: &ScreenShareSession) -> ScreenShareStateDto {
        let participants = session
            .participants
            .values()
            .map(|p| ParticipantDto {
                user_id: p.user_id,
                username: p.username.clone(),
                first_name: p.first_name.clone(),
                is_broadcasting: session.broadcaster_id == Some(p.user_id),
                is_connected: p.is_connected,
            })
            .collect();

        ScreenShareStateDto {
            session_id: session.id,
            code: session.code.clone(),
            status: session.status.as_str().to_string(),
            participants,
            broadcaster_id: session.broadcaster_id,
            is_swapping: session.swap_request.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_session_with_two_participants() -> (ScreenShareManager, Uuid, Uuid, Uuid) {
        let manager = ScreenShareManager::new();
        let session_id = Uuid::new_v4();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();

        manager
            .create_session(
                session_id,
                "ROOM1234".to_string(),
                user_a,
                "user_a".to_string(),
                "Alice".to_string(),
            )
            .await
            .expect("Falha ao criar sessão");

        manager
            .join_session(
                session_id,
                user_b,
                "user_b".to_string(),
                "Bob".to_string(),
            )
            .await
            .expect("Falha ao entrar na sessão");

        (manager, session_id, user_a, user_b)
    }

    #[tokio::test]
    async fn test_01_join_two_authorized_participants() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;
        let state = manager.get_session_state(session_id).await.unwrap();

        assert_eq!(state.participants.len(), 2);
        assert_eq!(state.status, "ready");
        assert!(state.participants.iter().any(|p| p.user_id == user_a));
        assert!(state.participants.iter().any(|p| p.user_id == user_b));
    }

    #[tokio::test]
    async fn test_02_reject_third_participant() {
        let (manager, session_id, _user_a, _user_b) = setup_session_with_two_participants().await;
        let user_c = Uuid::new_v4();

        let result = manager
            .join_session(session_id, user_c, "user_c".to_string(), "Charlie".to_string())
            .await;

        assert_eq!(result, Err(ScreenShareError::SessionFull));
    }

    #[tokio::test]
    async fn test_03_transmission_started_by_either_participant() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;

        // Inicia com participante A
        manager.start_broadcast(session_id, user_a).await.unwrap();
        let state_a = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(state_a.broadcaster_id, Some(user_a));

        // Encerra transmissão de A
        manager.stop_broadcast(session_id, user_a).await.unwrap();
        let state_stopped = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(state_stopped.broadcaster_id, None);
        assert_eq!(state_stopped.status, "ready");

        // Participante B inicia transmissão
        manager.start_broadcast(session_id, user_b).await.unwrap();
        let state_b = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(state_b.broadcaster_id, Some(user_b));
    }

    #[tokio::test]
    async fn test_04_swap_transmission_a_to_b() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;

        // A começa transmitindo
        manager.start_broadcast(session_id, user_a).await.unwrap();

        // B solicita troca
        manager.request_swap(session_id, user_b).await.unwrap();
        let state_swap = manager.get_session_state(session_id).await.unwrap();
        assert!(state_swap.is_swapping);

        // A aceita a troca
        manager.respond_swap(session_id, user_a, true).await.unwrap();

        let final_state = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(final_state.broadcaster_id, Some(user_b));
        assert!(!final_state.is_swapping);
    }

    #[tokio::test]
    async fn test_05_swap_transmission_b_to_a() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;

        // B começa transmitindo
        manager.start_broadcast(session_id, user_b).await.unwrap();

        // A solicita troca
        manager.request_swap(session_id, user_a).await.unwrap();
        let state_swap = manager.get_session_state(session_id).await.unwrap();
        assert!(state_swap.is_swapping);

        // B aceita a troca
        manager.respond_swap(session_id, user_b, true).await.unwrap();

        let final_state = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(final_state.broadcaster_id, Some(user_a));
        assert!(!final_state.is_swapping);
    }

    #[tokio::test]
    async fn test_06_simultaneous_broadcast_requests() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;

        // A inicia
        manager.start_broadcast(session_id, user_a).await.unwrap();

        // B tenta iniciar simultaneamente sem pedir troca
        let res_b = manager.start_broadcast(session_id, user_b).await;
        assert_eq!(res_b, Err(ScreenShareError::AnotherUserBroadcasting));

        // A tenta iniciar novamente enquanto já transmite
        let res_a = manager.start_broadcast(session_id, user_a).await;
        assert_eq!(res_a, Err(ScreenShareError::AlreadyBroadcasting));
    }

    #[tokio::test]
    async fn test_07_reject_state_change_by_unauthorized_user() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;
        let stranger = Uuid::new_v4();

        // Usuário de fora tenta iniciar transmissão
        let res = manager.start_broadcast(session_id, stranger).await;
        assert_eq!(res, Err(ScreenShareError::UserNotFound));

        // Usuário de fora tenta pedir troca
        let res = manager.request_swap(session_id, stranger).await;
        assert_eq!(res, Err(ScreenShareError::UserNotFound));

        // Usuário de fora tenta fechar a sessão
        let res = manager.close_session(session_id, stranger).await;
        assert_eq!(res, Err(ScreenShareError::Unauthorized));

        // A está transmitindo, B tenta interromper transmissão de A (não autorizado)
        manager.start_broadcast(session_id, user_a).await.unwrap();
        let res = manager.stop_broadcast(session_id, user_b).await;
        assert_eq!(res, Err(ScreenShareError::Unauthorized));
    }

    #[tokio::test]
    async fn test_08_disconnect_during_swap() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;

        // Registra canais WebSocket
        let (tx_a, _rx_a) = tokio::sync::mpsc::unbounded_channel();
        let (tx_b, _rx_b) = tokio::sync::mpsc::unbounded_channel();
        manager.attach_ws(session_id, user_a, tx_a).await.unwrap();
        manager.attach_ws(session_id, user_b, tx_b).await.unwrap();

        // A transmite, B pede troca
        manager.start_broadcast(session_id, user_a).await.unwrap();
        manager.request_swap(session_id, user_b).await.unwrap();

        // B desconecta durante a troca
        manager.detach_ws(session_id, user_b).await.unwrap();

        let state = manager.get_session_state(session_id).await.unwrap();
        // A solicitação de troca deve ter sido cancelada
        assert!(!state.is_swapping);
        // A sessão continua ativa
        assert_ne!(state.status, "closed");
        // A continua como transmissor
        assert_eq!(state.broadcaster_id, Some(user_a));
    }

    #[tokio::test]
    async fn test_09_stop_broadcast_without_closing_session() {
        let (manager, session_id, user_a, _user_b) = setup_session_with_two_participants().await;

        manager.start_broadcast(session_id, user_a).await.unwrap();
        manager.stop_broadcast(session_id, user_a).await.unwrap();

        let state = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(state.broadcaster_id, None);
        assert_eq!(state.status, "ready");
    }

    #[tokio::test]
    async fn test_10_close_session_and_state_cleanup() {
        let (manager, session_id, user_a, user_b) = setup_session_with_two_participants().await;

        manager.close_session(session_id, user_a).await.unwrap();

        let state = manager.get_session_state(session_id).await.unwrap();
        assert_eq!(state.status, "closed");

        // Código de convite foi invalidado
        assert_eq!(manager.resolve_code("ROOM1234").await, None);

        // Nenhuma nova ação é aceita
        let res_join = manager
            .join_session(session_id, Uuid::new_v4(), "c".to_string(), "C".to_string())
            .await;
        assert_eq!(res_join, Err(ScreenShareError::SessionClosed));

        let res_start = manager.start_broadcast(session_id, user_b).await;
        assert_eq!(res_start, Err(ScreenShareError::SessionClosed));
    }
}

