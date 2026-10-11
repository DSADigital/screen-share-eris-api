use actix_web::{HttpRequest, HttpResponse, Responder, post, web::Data};
use nanoid::nanoid;
use uuid::Uuid;

use crate::{
    handlers::websocket_url,
    identity::AuthenticatedUser,
    manager::ScreenShareManager,
    responses::{ApiResponse, internal_error},
    schemas::CreateScreenShareResponse,
};

#[post("/sessions")]
pub async fn create_session(
    user: AuthenticatedUser,
    req: HttpRequest,
    manager: Data<ScreenShareManager>,
) -> impl Responder {
    let session_id = Uuid::new_v4();
    let alphabet: [char; 32] = [
        '2', '3', '4', '5', '6', '7', '8', '9',
        'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
        'J', 'K', 'L', 'M', 'N', 'P', 'Q', 'R',
        'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    ];
    let code = nanoid!(8, &alphabet);

    // [DEMO] Em produção, aqui é inserido um registro de auditoria na tabela
    // `screen_share_sessions` com: id, code, created_by, participant_a e
    // status ('waiting'). Nenhum conteúdo de tela é persistido. Nesta cópia
    // não há banco de dados.

    // Inicialização da máquina de estados em memória
    let ticket = match manager
        .create_session(
            session_id,
            code.clone(),
            user.0.id,
            user.0.username.clone(),
            user.0.first_name.clone(),
        )
        .await
    {
        Ok(t) => t,
        Err(e) => {
            log::error!("{e}");
            return internal_error();
        }
    };

    HttpResponse::Ok().json(ApiResponse {
        status: "ok",
        message: "Sessão de compartilhamento criada com sucesso.",
        toast: "Sessão criada! Compartilhe o código para iniciar.",
        data: CreateScreenShareResponse {
            session_id,
            code,
            ticket,
            websocket_url: websocket_url(&req),
        },
    })
}
