use actix_web::{
    HttpRequest, HttpResponse, Responder, post,
    web::{Data, Json, Path},
};
use uuid::Uuid;

use crate::{
    handlers::websocket_url,
    identity::AuthenticatedUser,
    manager::{ScreenShareError, ScreenShareManager},
    responses::{ApiResponse, bad_request, internal_error},
    schemas::{JoinScreenShareRequest, JoinScreenShareResponse},
};

#[post("/sessions/{id}/join")]
pub async fn join_session(
    user: AuthenticatedUser,
    path_id: Path<String>,
    body: Option<Json<JoinScreenShareRequest>>,
    req: HttpRequest,
    manager: Data<ScreenShareManager>,
) -> impl Responder {
    let target = path_id.into_inner();

    // Tenta interpretar o target como UUID ou resolve como código alfanumérico
    let session_id = if let Ok(uuid) = Uuid::parse_str(&target) {
        uuid
    } else if let Some(resolved_id) = manager.resolve_code(&target).await {
        resolved_id
    } else if let Some(ref b) = body {
        if let Some(ref code) = b.code {
            if let Some(resolved_id) = manager.resolve_code(code).await {
                resolved_id
            } else {
                return HttpResponse::BadRequest().json(ApiResponse::<()> {
                    status: "not_found",
                    message: "Sessão não encontrada com o código informado.",
                    toast: "Código de sessão inválido.",
                    data: (),
                });
            }
        } else {
            return bad_request();
        }
    } else {
        return HttpResponse::BadRequest().json(ApiResponse::<()> {
            status: "not_found",
            message: "Sessão não encontrada com o identificador informado.",
            toast: "Sessão não encontrada.",
            data: (),
        });
    };

    // Tenta ingressar na máquina de estados da sessão
    let ticket = match manager
        .join_session(
            session_id,
            user.0.id,
            user.0.username.clone(),
            user.0.first_name.clone(),
        )
        .await
    {
        Ok(t) => t,
        Err(ScreenShareError::SessionFull) => {
            return HttpResponse::BadRequest().json(ApiResponse::<()> {
                status: "session_full",
                message: "A sessão já possui o limite máximo de 2 participantes conectados.",
                toast: "Esta sessão já atingiu o limite de participantes.",
                data: (),
            });
        }
        Err(ScreenShareError::SessionClosed) => {
            return HttpResponse::BadRequest().json(ApiResponse::<()> {
                status: "session_closed",
                message: "Esta sessão de compartilhamento de tela já foi encerrada.",
                toast: "A sessão foi encerrada.",
                data: (),
            });
        }
        Err(ScreenShareError::SessionNotFound) => {
            return HttpResponse::BadRequest().json(ApiResponse::<()> {
                status: "not_found",
                message: "Sessão não encontrada.",
                toast: "Sessão não encontrada.",
                data: (),
            });
        }
        Err(e) => {
            log::error!("{e}");
            return internal_error();
        }
    };

    // [DEMO] Em produção, o registro de auditoria em `screen_share_sessions`
    // é atualizado com participant_b e status ('ready'). Sem banco aqui.

    let state = match manager.get_session_state(session_id).await {
        Ok(s) => s,
        Err(e) => {
            log::error!("{e}");
            return internal_error();
        }
    };

    HttpResponse::Ok().json(ApiResponse {
        status: "ok",
        message: "Ingresso na sessão realizado com sucesso.",
        toast: "Conectado à sessão de compartilhamento!",
        data: JoinScreenShareResponse {
            session_id,
            code: state.code,
            ticket,
            websocket_url: websocket_url(&req),
            status: state.status,
            current_broadcaster: state.broadcaster_id,
        },
    })
}
