pub mod auth;
pub mod dashboard;
pub mod deployments;
pub mod projects;
pub mod settings;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::AppState;

/// Build the complete axum router with all routes.
pub fn build_router(state: AppState) -> Router {
    // ---- Public routes (no auth) ----
    let public = Router::new()
        .route("/setup", get(auth::get_setup).post(auth::post_setup))
        .route("/login", get(auth::get_login).post(auth::post_login))
        .with_state(state.clone());

    // ---- Webhook routes (no auth — verified via HMAC/shared-secret) ----
    let webhooks = Router::new()
        .route("/hook/{slug}", post(crate::webhooks::handle_webhook))
        .with_state(state.clone());

    // ---- Protected routes (require auth + CSRF) ----
    let protected = Router::new()
        // Dashboard
        .route("/", get(dashboard::dashboard))
        .route("/dashboard", get(dashboard::dashboard))
        .route("/dashboard/stats", get(dashboard::dashboard_stats))
        .route("/dashboard/telemetry", get(dashboard::dashboard_telemetry))
        .route(
            "/dashboard/recent-deploys",
            get(dashboard::dashboard_recent_deploys),
        )
        // Projects
        .route(
            "/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route("/projects/new", get(projects::new_project_form))
        .route("/projects/new/autodetect", get(projects::autodetect))
        .route("/projects/{slug}", get(projects::project_detail))
        .route("/projects/{slug}/deploy", post(projects::deploy_now))
        .route("/projects/{slug}/stop", post(projects::stop_project))
        .route("/projects/{slug}/restart", post(projects::restart_project))
        .route(
            "/projects/{slug}/rollback",
            get(projects::rollback_picker).post(projects::rollback_to),
        )
        .route("/projects/{slug}/remove", post(projects::remove_project))
        .route(
            "/projects/{slug}/webhook-toggle",
            post(projects::toggle_webhook),
        )
        .route(
            "/projects/{slug}/compose",
            get(projects::get_compose).post(projects::save_compose),
        )
        .route(
            "/projects/{slug}/env",
            get(projects::get_env).post(projects::save_env),
        )
        .route(
            "/projects/{slug}/env/example",
            get(projects::get_env_example),
        )
        // Deployments
        .route("/deployments", get(deployments::deployments))
        .route("/deployments/{id}", get(deployments::deploy_detail))
        .route(
            "/deployments/{id}/log",
            get(deployments::deploy_log_fragment),
        )
        .route(
            "/deployments/{id}/log/full",
            get(deployments::deploy_log_full),
        )
        // Settings
        .route(
            "/settings",
            get(settings::settings_page).post(settings::save_settings),
        )
        .route("/settings/ssh/generate", post(settings::generate_ssh_key))
        .route("/settings/ssh-mode", post(settings::save_ssh_mode))
        .route("/settings/github-token", post(settings::save_github_token))
        .route(
            "/settings/github-token/remove",
            post(settings::remove_github_token),
        )
        .route("/settings/docker", get(settings::docker_maintain))
        .route(
            "/settings/docker/prune-safe",
            post(settings::docker_prune_safe),
        )
        .route(
            "/settings/docker/prune-nuclear",
            post(settings::docker_prune_nuclear),
        )
        .route("/settings/change-password", post(settings::change_password))
        .route(
            "/settings/sign-out-everywhere",
            post(settings::sign_out_everywhere),
        )
        // Logout (protected — requires session)
        .route("/logout", post(auth::post_logout))
        // Auth middleware applied before with_state (axum's documented pattern
        // for from_fn_with_state).
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_auth,
        ))
        .with_state(state.clone());

    // Combine: public + webhooks run without auth; protected has the middleware layer.
    Router::new()
        .merge(public)
        .merge(webhooks)
        .merge(protected)
        .with_state(state)
}
