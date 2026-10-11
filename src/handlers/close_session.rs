use actix_web::{
    HttpResponse, Responder, delete,
    web::{Data, Path},
};
use uuid::Uuid;

use crate::{
    identity::AuthenticatedUser,
    manager::{ScreenShareError, ScreenShareManager},
    responses::{ApiResponse, CleanResponse, internal_error, unauthorized},
};

#[delete("/sessions/{id}")]
pub async fn close_session(
    user: AuthenticatedUser,
    path_id: Path<Uuid>,
    manager: Data<ScreenShareManager>,
) -> impl Responder {
    let session_id = path_id.into_inner();

    if let Err(err) = manager.close_session(session_id, user.0.id).await {
        match err {
            ScreenShareError::Unauthorized => return unauthorized(),
            ScreenShareError::SessionNotFound => {
                return HttpResponse::NotFound().json(ApiResponse::<()> {
                    status: "not_found",
                    message: "Sessão não encontrada.",
                    toast: "Sessão não encontrada.",
                    data: (),
                });
            }
            e => {
                log::error!("{e}");
                return internal_error();
            }
        }
    }

    // [DEMO] Em produção, o registro de auditoria recebe status 'closed',
    // active_broadcaster = NULL e ended_at = now(). Sem banco aqui.

    HttpResponse::Ok().json(ApiResponse {
        status: "ok",
        message: "Sessão de compartilhamento encerrada com sucesso.",
        toast: "Sessão finalizada e recursos liberados.",
        data: CleanResponse {},
    })
}
