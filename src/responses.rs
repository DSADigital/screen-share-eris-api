//! Envelope de resposta HTTP — mesmo formato usado em toda a Éris API.

use actix_web::HttpResponse;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub status: &'static str,
    pub message: &'static str,
    pub toast: &'static str,
    pub data: T,
}

#[derive(Debug, Serialize)]
pub struct CleanResponse {}

pub fn bad_request() -> HttpResponse {
    HttpResponse::BadRequest().json(ApiResponse {
        status: "bad_request",
        message: "Requisição inválida.",
        toast: "Requisição inválida.",
        data: (),
    })
}

pub fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(ApiResponse {
        status: "unauthorized",
        message: "Não autorizado.",
        toast: "Você não tem permissão para esta ação.",
        data: (),
    })
}

pub fn internal_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(ApiResponse {
        status: "internal_error",
        message: "Erro interno do servidor.",
        toast: "Algo deu errado. Tente novamente.",
        data: (),
    })
}
