//! Canal WebSocket de sinalização WebRTC.
//!
//! CÓPIA FIEL de `src/services/screen_share/services/ws_signaling.rs` da
//! Éris API (apenas imports e a macro de log foram adaptados).
//!
//! Repare: o servidor só trafega JSON de controle (start/stop/swap) e
//! sinalização (SDP/ICE). O vídeo da tela flui diretamente entre os dois
//! navegadores via WebRTC (peer-to-peer, criptografado com DTLS-SRTP).

use actix_web::{
    Error, HttpRequest, HttpResponse, get,
    web::{Data, Payload, Query},
};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    manager::ScreenShareManager,
    schemas::{ClientSignalMessage, ServerSignalMessage},
};

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    pub ticket: Option<String>,
}

#[get("/ws")]
pub async fn ws_signaling(
    req: HttpRequest,
    stream: Payload,
    query: Query<WsQuery>,
    manager: Data<ScreenShareManager>,
) -> Result<HttpResponse, Error> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, stream)?;

    let manager_clone = manager.get_ref().clone();
    let initial_ticket = query.ticket.clone();

    actix_web::rt::spawn(async move {
        // Autenticação inicial
        let (session_id, user_id) = match initial_ticket {
            Some(ref ticket) => match manager_clone.consume_ticket(ticket).await {
                Ok(ids) => ids,
                Err(_) => {
                    let err_msg = ServerSignalMessage::Error {
                        code: "invalid_ticket".to_string(),
                        message: "Ticket de autenticação inválido ou expirado.".to_string(),
                    };
                    if let Ok(json) = serde_json::to_string(&err_msg) {
                        let _ = session.text(json).await;
                    }
                    let _ = session.close(None).await;
                    return;
                }
            },
            None => {
                // Aguarda a primeira mensagem ser ClientSignalMessage::Join { ticket }
                let mut authenticated_ids: Option<(Uuid, Uuid)> = None;
                while let Some(msg) = msg_stream.next().await {
                    match msg {
                        Ok(actix_ws::Message::Text(text)) => {
                            if let Ok(ClientSignalMessage::Join { ticket }) =
                                serde_json::from_str::<ClientSignalMessage>(&text)
                            {
                                match manager_clone.consume_ticket(&ticket).await {
                                    Ok(ids) => {
                                        authenticated_ids = Some(ids);
                                        break;
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                        Ok(actix_ws::Message::Ping(bytes)) => {
                            let _ = session.pong(&bytes).await;
                        }
                        _ => break,
                    }
                }

                match authenticated_ids {
                    Some(ids) => ids,
                    None => {
                        let err_msg = ServerSignalMessage::Error {
                            code: "unauthorized".to_string(),
                            message: "Autenticação obrigatória com ticket válido.".to_string(),
                        };
                        if let Ok(json) = serde_json::to_string(&err_msg) {
                            let _ = session.text(json).await;
                        }
                        let _ = session.close(None).await;
                        return;
                    }
                }
            }
        };

        // Cria o canal de mensagens enviadas do servidor para o cliente
        let (tx, mut rx) = mpsc::unbounded_channel::<ServerSignalMessage>();

        // Registra o WebSocket do participante no gerenciador
        let state = match manager_clone.attach_ws(session_id, user_id, tx).await {
            Ok(s) => s,
            Err(e) => {
                let err_msg = ServerSignalMessage::Error {
                    code: "session_attach_failed".to_string(),
                    message: e.to_string(),
                };
                if let Ok(json) = serde_json::to_string(&err_msg) {
                    let _ = session.text(json).await;
                }
                let _ = session.close(None).await;
                return;
            }
        };

        // Envia confirmação de autenticação e estado da sessão
        let auth_msg = ServerSignalMessage::Authenticated {
            user_id,
            session_id,
        };
        if let Ok(json) = serde_json::to_string(&auth_msg) {
            let _ = session.text(json).await;
        }

        let state_msg = ServerSignalMessage::SessionState(state);
        if let Ok(json) = serde_json::to_string(&state_msg) {
            let _ = session.text(json).await;
        }

        // Loop principal de comunicação assíncrona
        let mut interval = actix_web::rt::time::interval(std::time::Duration::from_secs(20));

        loop {
            tokio::select! {
                // Heartbeat periódica para detecção de liveness
                _ = interval.tick() => {
                    if session.ping(b"heartbeat").await.is_err() {
                        break;
                    }
                }

                // Mensagens que chegam do gerenciador para serem entregues a este cliente
                Some(outgoing) = rx.recv() => {
                    let json = match serde_json::to_string(&outgoing) {
                        Ok(j) => j,
                        Err(e) => {
                            log::error!("{e}");
                            continue;
                        }
                    };

                    if session.text(json).await.is_err() {
                        break;
                    }

                    if matches!(outgoing, ServerSignalMessage::SessionClosed { .. }) {
                        let _ = session.close(None).await;
                        break;
                    }
                }

                // Mensagens recebidas do cliente via WebSocket
                msg = msg_stream.next() => {
                    match msg {
                        Some(Ok(actix_ws::Message::Text(text))) => {
                            let parsed = match serde_json::from_str::<ClientSignalMessage>(&text) {
                                Ok(m) => m,
                                Err(e) => {
                                    let err_msg = ServerSignalMessage::Error {
                                        code: "bad_message_format".to_string(),
                                        message: format!("Payload JSON inválido: {}", e),
                                    };
                                    if let Ok(json) = serde_json::to_string(&err_msg) {
                                        let _ = session.text(json).await;
                                    }
                                    continue;
                                }
                            };

                            match parsed {
                                ClientSignalMessage::Join { .. } => {
                                    // Já autenticado
                                }
                                ClientSignalMessage::Ping => {
                                    if let Ok(json) = serde_json::to_string(&ServerSignalMessage::Pong) {
                                        let _ = session.text(json).await;
                                    }
                                }
                                ClientSignalMessage::StartBroadcast => {
                                    if let Err(err) = manager_clone.start_broadcast(session_id, user_id).await {
                                        let err_msg = ServerSignalMessage::Error {
                                            code: "start_broadcast_failed".to_string(),
                                            message: err.to_string(),
                                        };
                                        if let Ok(json) = serde_json::to_string(&err_msg) {
                                            let _ = session.text(json).await;
                                        }
                                    }
                                }
                                ClientSignalMessage::StopBroadcast => {
                                    if let Err(err) = manager_clone.stop_broadcast(session_id, user_id).await {
                                        let err_msg = ServerSignalMessage::Error {
                                            code: "stop_broadcast_failed".to_string(),
                                            message: err.to_string(),
                                        };
                                        if let Ok(json) = serde_json::to_string(&err_msg) {
                                            let _ = session.text(json).await;
                                        }
                                    }
                                }
                                ClientSignalMessage::RequestSwap => {
                                    if let Err(err) = manager_clone.request_swap(session_id, user_id).await {
                                        let err_msg = ServerSignalMessage::Error {
                                            code: "request_swap_failed".to_string(),
                                            message: err.to_string(),
                                        };
                                        if let Ok(json) = serde_json::to_string(&err_msg) {
                                            let _ = session.text(json).await;
                                        }
                                    }
                                }
                                ClientSignalMessage::RespondSwap { accept } => {
                                    if let Err(err) = manager_clone.respond_swap(session_id, user_id, accept).await {
                                        let err_msg = ServerSignalMessage::Error {
                                            code: "respond_swap_failed".to_string(),
                                            message: err.to_string(),
                                        };
                                        if let Ok(json) = serde_json::to_string(&err_msg) {
                                            let _ = session.text(json).await;
                                        }
                                    }
                                }
                                ClientSignalMessage::SdpOffer { sdp } => {
                                    let msg = ServerSignalMessage::SdpOffer {
                                        sdp,
                                        from_user_id: user_id,
                                    };
                                    let _ = manager_clone.route_signaling(session_id, user_id, msg).await;
                                }
                                ClientSignalMessage::SdpAnswer { sdp } => {
                                    let msg = ServerSignalMessage::SdpAnswer {
                                        sdp,
                                        from_user_id: user_id,
                                    };
                                    let _ = manager_clone.route_signaling(session_id, user_id, msg).await;
                                }
                                ClientSignalMessage::IceCandidate { candidate, sdp_mid, sdp_m_line_index } => {
                                    let msg = ServerSignalMessage::IceCandidate {
                                        candidate,
                                        sdp_mid,
                                        sdp_m_line_index,
                                        from_user_id: user_id,
                                    };
                                    let _ = manager_clone.route_signaling(session_id, user_id, msg).await;
                                }
                            }
                        }
                        Some(Ok(actix_ws::Message::Ping(bytes))) => {
                            let _ = session.pong(&bytes).await;
                        }
                        Some(Ok(actix_ws::Message::Close(reason))) => {
                            let _ = session.close(reason).await;
                            break;
                        }
                        None => {
                            // Conexão encerrada pelo cliente
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Limpeza de conexão e recursos
        let _ = manager_clone.detach_ws(session_id, user_id).await;
    });

    Ok(response)
}
