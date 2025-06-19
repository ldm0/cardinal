// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
async fn search(query: &str) -> Result<Vec<String>, String> {
    println!("Searching for: {}", query);
    // For now, return mock data
    Ok(vec![
        "/Users/test/file1.txt".to_string(),
        "/Users/test/another/file2.docx".to_string(),
        "/Users/test/folder/image.png".to_string(),
    ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![search])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
