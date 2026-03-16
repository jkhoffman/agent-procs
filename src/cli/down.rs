pub async fn execute(session: &str) -> i32 {
    crate::cli::stop::execute_all(session).await
}
