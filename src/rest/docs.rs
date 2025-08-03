use super::*;

pub fn docs_routes(state: Arc<Server>) -> ApiRouter {
    let router: ApiRouter = ApiRouter::new()
        .route("/", get(Scalar::new("/docs/private/api.json").with_title("OpenAPI").axum_handler()))
        .route("/private/api.json", get(serve_docs))
        .with_state(state);

    router
}

async fn serve_docs(Extension(api): Extension<Arc<OpenApi>>) -> impl IntoApiResponse {
    Json(api).into_response()
}
