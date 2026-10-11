//! screen-share-eris-api
//!
//! Cópia pública e funcional do serviço de compartilhamento de tela da
//! Éris API (DSA Digital), publicada para auditoria aberta.
//!
//! - Sem banco de dados, sem analytics, sem chamadas a serviços externos.
//! - Todo estado vive em memória e some quando o processo é encerrado.
//! - O vídeo nunca passa pelo servidor: ele é transmitido P2P via WebRTC.

mod handlers;
mod identity;
mod manager;
mod responses;
mod schemas;

use std::env;

use actix_files::Files;
use actix_web::{
    App, HttpServer,
    middleware::{DefaultHeaders, Logger},
    web::Data,
};

use crate::{identity::IdentityStore, manager::ScreenShareManager};

/// Content-Security-Policy da página de demo: bloqueia qualquer script,
/// estilo, fonte ou conexão para domínios de terceiros.
const CSP: &str = "default-src 'self'; \
    script-src 'self'; \
    style-src 'self'; \
    img-src 'self' data:; \
    media-src 'self' blob:; \
    connect-src 'self' ws: wss:; \
    frame-ancestors 'none'; \
    base-uri 'none'; \
    form-action 'none'";

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let manager = Data::new(ScreenShareManager::new());
    let identities = Data::new(IdentityStore::new());
    let static_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/static");

    log::info!("Screen share demo disponível em http://{host}:{port}");

    HttpServer::new(move || {
        App::new()
            .app_data(manager.clone())
            .app_data(identities.clone())
            // Log de acesso SEM IP, user-agent, referer ou query string (que
            // carregaria o ticket do WebSocket): apenas método, caminho,
            // status e duração.
            .wrap(
                Logger::new("%{method}xi %U -> %s (%Dms)")
                    .custom_request_replace("method", |req| req.method().to_string()),
            )
            .wrap(
                DefaultHeaders::new()
                    .add(("Content-Security-Policy", CSP))
                    .add(("Referrer-Policy", "no-referrer"))
                    .add(("X-Content-Type-Options", "nosniff"))
                    .add(("Permissions-Policy", "camera=(), microphone=(), geolocation=(), display-capture=(self)")),
            )
            .configure(handlers::routes)
            .service(Files::new("/", static_dir).index_file("index.html"))
    })
    .bind((host.as_str(), port))?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    use actix_web::{App, test, web::Data};
    use serde_json::{Value, json};

    use crate::{handlers, identity::IdentityStore, manager::ScreenShareManager};

    macro_rules! app {
        () => {
            test::init_service(
                App::new()
                    .app_data(Data::new(ScreenShareManager::new()))
                    .app_data(Data::new(IdentityStore::new()))
                    .configure(handlers::routes),
            )
            .await
        };
    }

    macro_rules! identity {
        ($app:expr, $name:expr) => {{
            let req = test::TestRequest::post()
                .uri("/api/demo/identity")
                .set_json(json!({ "first_name": $name }))
                .to_request();
            let body: Value = test::call_and_read_body_json(&$app, req).await;
            body["data"]["token"].as_str().unwrap().to_string()
        }};
    }

    #[actix_web::test]
    async fn http_flow_create_join_and_reject_third() {
        let app = app!();
        let alice = identity!(app, "Alice");
        let bob = identity!(app, "Bob");
        let carol = identity!(app, "Carol");

        // Sem token → 401
        let req = test::TestRequest::post().uri("/api/screen-share/sessions").to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 401);

        // Alice cria
        let req = test::TestRequest::post()
            .uri("/api/screen-share/sessions")
            .insert_header(("Authorization", format!("Bearer {alice}")))
            .to_request();
        let created: Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(created["status"], "ok");
        let code = created["data"]["code"].as_str().unwrap().to_string();
        assert_eq!(code.len(), 8);
        assert!(created["data"]["ticket"].as_str().unwrap().len() == 32);

        // Bob entra pelo código
        let req = test::TestRequest::post()
            .uri(&format!("/api/screen-share/sessions/{code}/join"))
            .insert_header(("Authorization", format!("Bearer {bob}")))
            .to_request();
        let joined: Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(joined["data"]["status"], "ready");

        // Carol é recusada (máximo de 2)
        let req = test::TestRequest::post()
            .uri(&format!("/api/screen-share/sessions/{code}/join"))
            .insert_header(("Authorization", format!("Bearer {carol}")))
            .to_request();
        let full: Value = test::call_and_read_body_json(&app, req).await;
        assert_eq!(full["status"], "session_full");

        // Carol não pode ler o estado nem encerrar a sessão
        let session_id = created["data"]["session_id"].as_str().unwrap();
        let req = test::TestRequest::get()
            .uri(&format!("/api/screen-share/sessions/{session_id}"))
            .insert_header(("Authorization", format!("Bearer {carol}")))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 401);

        let req = test::TestRequest::delete()
            .uri(&format!("/api/screen-share/sessions/{session_id}"))
            .insert_header(("Authorization", format!("Bearer {carol}")))
            .to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 401);
    }
}
