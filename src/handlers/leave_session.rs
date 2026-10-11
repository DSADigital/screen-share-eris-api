use actix_web::{
    HttpResponse, Responder, post,
    web::{Data, Path},
};
use uuid::Uuid;

use crate::{
    identity::AuthenticatedUser,
    manager::ScreenShareManager,
    responses::{ApiResponse, CleanResponse, internal_error},
};

#[post("/sessions/{id}/leave")]
pub async fn leave_session(
    user: AuthenticatedUser,
    path_id: Path<Uuid>,
    manager: Data<ScreenShareManager>,
) -> impl Responder {
    let session_id = path_id.into_inner();

    if let Err(e) = manager.leave_session(session_id, user.0.id).await {
        log::error!("{e}");
        return internal_error();
    }

    // [DEMO] Em produção, o registro de auditoria limpa participant_b /
    // active_broadcaster e volta o status para 'waiting'. Sem banco aqui.

    HttpResponse::Ok().json(ApiResponse {
        status: "ok",
        message: "Você saiu da sessão de compartilhamento.",
        toast: "Você desconectou-se da sessão.",
        data: CleanResponse {},
    })
}
