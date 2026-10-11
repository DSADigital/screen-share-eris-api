//! [DEMO] Endpoint exclusivo desta cópia: cria uma identidade descartável.
//! Em produção, o usuário já chega autenticado via JWT.

use actix_web::{
    HttpResponse, Responder, post,
    web::{Data, Json},
};

use crate::{
    identity::IdentityStore,
    responses::{ApiResponse, internal_error},
    schemas::{CreateIdentityRequest, CreateIdentityResponse},
};

#[post("/api/demo/identity")]
pub async fn create_identity(
    body: Json<CreateIdentityRequest>,
    store: Data<IdentityStore>,
) -> impl Responder {
    let first_name: String = body.first_name.trim().chars().take(32).collect();
    if first_name.is_empty() {
        return HttpResponse::BadRequest().json(ApiResponse {
            status: "bad_request",
            message: "Informe um nome para exibição.",
            toast: "Informe um nome para exibição.",
            data: (),
        });
    }

    let Some((token, user)) = store.issue(first_name) else {
        return internal_error();
    };

    HttpResponse::Ok().json(ApiResponse {
        status: "ok",
        message: "Identidade temporária criada.",
        toast: "",
        data: CreateIdentityResponse {
            token,
            user_id: user.id,
            username: user.username,
            first_name: user.first_name,
        },
    })
}
