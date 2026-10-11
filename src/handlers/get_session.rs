use actix_web::{
    HttpResponse, Responder, get,
    web::{Data, Path},
};
use uuid::Uuid;

use crate::{
    identity::AuthenticatedUser,
    manager::{ScreenShareError, ScreenShareManager},
    responses::{ApiResponse, internal_error, unauthorized},
};

#[get("/sessions/{id}")]
pub async fn get_session(
    user: AuthenticatedUser,
    path_id: Path<Uuid>,
    manager: Data<ScreenShareManager>,
) -> impl Responder {
    let session_id = path_id.into_inner();

    let state = match manager.get_session_state(session_id).await {
        Ok(s) => s,
        Err(ScreenShareError::SessionNotFound) => {
            return HttpResponse::NotFound().json(ApiResponse::<()> {
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

    // Valida se o solicitante é um dos participantes autorizados
    let is_participant = state.participants.iter().any(|p| p.user_id == user.0.id);
    if !is_participant {
        return unauthorized();
    }

    HttpResponse::Ok().json(ApiResponse {
        status: "ok",
        message: "Estado da sessão obtido com sucesso.",
        toast: "",
        data: state,
    })
}
