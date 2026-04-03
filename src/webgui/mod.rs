use axum::response::Html;

pub async fn index() -> Html<String> {
    // Deliver the HTML file
    Html(include_str!("../../assets/html/index.html").to_string())
}
