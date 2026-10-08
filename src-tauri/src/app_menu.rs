//! The menu bar, in the interface's language. Tauri's default menu and
//! muda's predefined items are English only, so the menu is built here from
//! one label table; the frontend names the language at startup and on every
//! change, and the menu is rebuilt.
//!
//! English passes no text for predefined items, so its menu is exactly the
//! default one (muda appends the app name to About, Hide and Quit itself).

use serde::Deserialize;

/// The interface languages a menu exists for. Mirrors `UiLanguage` in
/// `src/preferences.ts`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum UiLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

/// Every word the menu bar shows. `None` keeps the predefined item's own
/// title. The application submenu (About, Services, Hide, Quit) and View
/// exist on macOS only, so elsewhere some words go unread.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct MenuLabels {
    pub about: Option<String>,
    pub services: Option<String>,
    pub hide: Option<String>,
    pub hide_others: Option<String>,
    pub quit: String,
    pub file: String,
    pub close_window: Option<String>,
    pub edit: String,
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub cut: Option<String>,
    pub copy: Option<String>,
    pub paste: Option<String>,
    pub select_all: Option<String>,
    pub view: String,
    pub fullscreen: Option<String>,
    pub window: String,
    pub minimize: Option<String>,
    pub maximize: Option<String>,
    pub help: String,
}

/// The macOS menu words of each language: the titles Apple's own apps use in
/// Simplified Chinese.
pub fn menu_labels(language: UiLanguage, app_name: &str) -> MenuLabels {
    match language {
        UiLanguage::English => MenuLabels {
            about: None,
            services: None,
            hide: None,
            hide_others: None,
            quit: format!("Quit {app_name}"),
            file: "File".into(),
            close_window: None,
            edit: "Edit".into(),
            undo: None,
            redo: None,
            cut: None,
            copy: None,
            paste: None,
            select_all: None,
            view: "View".into(),
            fullscreen: None,
            window: "Window".into(),
            minimize: None,
            maximize: None,
            help: "Help".into(),
        },
        UiLanguage::SimplifiedChinese => MenuLabels {
            about: Some(format!("关于 {app_name}")),
            services: Some("服务".into()),
            hide: Some(format!("隐藏 {app_name}")),
            hide_others: Some("隐藏其他".into()),
            quit: format!("退出 {app_name}"),
            file: "文件".into(),
            close_window: Some("关闭窗口".into()),
            edit: "编辑".into(),
            undo: Some("撤销".into()),
            redo: Some("重做".into()),
            cut: Some("剪切".into()),
            copy: Some("拷贝".into()),
            paste: Some("粘贴".into()),
            select_all: Some("全选".into()),
            view: "显示".into(),
            fullscreen: Some("进入全屏幕".into()),
            window: "窗口".into(),
            minimize: Some("最小化".into()),
            maximize: Some("缩放".into()),
            help: "帮助".into(),
        },
    }
}

/// The app menu's Quit item. Tauri's predefined Quit sends `terminate:`
/// straight to the process, so ⌘Q and Mallard › Quit skipped the frontend's
/// "Sync in progress" guard, which only hears window close requests. A plain
/// item with the same shortcut lets the handler route Quit through that
/// close request instead; the guard then decides, and the app exits when its
/// last window is gone.
#[cfg(not(test))]
const QUIT_MENU_ID: &str = "mallard-quit";

/// Tauri's default menu, item for item, in `language`, with Quit swapped for
/// the guarded one on macOS.
#[cfg(not(test))]
pub fn build(
    app: &tauri::AppHandle,
    language: UiLanguage,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{
        AboutMetadata, Menu, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
    };
    let pkg_info = app.package_info();
    let config = app.config();
    let labels = menu_labels(language, &pkg_info.name);
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };
    let window_menu = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        &labels.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, labels.minimize.as_deref())?,
            &PredefinedMenuItem::maximize(app, labels.maximize.as_deref())?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, labels.close_window.as_deref())?,
        ],
    )?;
    let help_menu = Submenu::with_id_and_items(
        app,
        HELP_SUBMENU_ID,
        &labels.help,
        true,
        &[
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::about(app, labels.about.as_deref(), Some(about_metadata.clone()))?,
        ],
    )?;
    #[cfg(target_os = "macos")]
    let quit =
        tauri::menu::MenuItem::with_id(app, QUIT_MENU_ID, &labels.quit, true, Some("CmdOrCtrl+Q"))?;
    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &Submenu::with_items(
                app,
                pkg_info.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(app, labels.about.as_deref(), Some(about_metadata))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, labels.services.as_deref())?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, labels.hide.as_deref())?,
                    &PredefinedMenuItem::hide_others(app, labels.hide_others.as_deref())?,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?,
            #[cfg(not(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            )))]
            &Submenu::with_items(
                app,
                &labels.file,
                true,
                &[
                    &PredefinedMenuItem::close_window(app, labels.close_window.as_deref())?,
                    #[cfg(not(target_os = "macos"))]
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                &labels.edit,
                true,
                &[
                    &PredefinedMenuItem::undo(app, labels.undo.as_deref())?,
                    &PredefinedMenuItem::redo(app, labels.redo.as_deref())?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, labels.cut.as_deref())?,
                    &PredefinedMenuItem::copy(app, labels.copy.as_deref())?,
                    &PredefinedMenuItem::paste(app, labels.paste.as_deref())?,
                    &PredefinedMenuItem::select_all(app, labels.select_all.as_deref())?,
                ],
            )?,
            #[cfg(target_os = "macos")]
            &Submenu::with_items(
                app,
                &labels.view,
                true,
                &[&PredefinedMenuItem::fullscreen(
                    app,
                    labels.fullscreen.as_deref(),
                )?],
            )?,
            &window_menu,
            &help_menu,
        ],
    )
}

#[cfg(not(test))]
pub fn on_menu_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    use tauri::Manager as _;
    if event.id() != QUIT_MENU_ID {
        return;
    }
    let windows = app.webview_windows();
    if windows.is_empty() {
        app.exit(0);
        return;
    }
    // A close request, not a destroy: the frontend's onCloseRequested
    // listener (ProjectSync.tsx) may prevent it while a sync is writing.
    for window in windows.values() {
        let _ = window.close();
    }
}

/// The frontend names its language at startup and whenever it changes; the
/// menu bar follows.
#[cfg(not(test))]
#[tauri::command]
pub fn set_ui_language(app: tauri::AppHandle, language: UiLanguage) -> Result<(), String> {
    let menu = build(&app, language).map_err(|error| error.to_string())?;
    app.set_menu(menu).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_keeps_every_default_title() {
        let labels = menu_labels(UiLanguage::English, "Mallard");
        assert_eq!(labels.quit, "Quit Mallard");
        assert_eq!(
            [
                &labels.file,
                &labels.edit,
                &labels.view,
                &labels.window,
                &labels.help
            ],
            ["File", "Edit", "View", "Window", "Help"]
        );
        for predefined in [
            &labels.about,
            &labels.services,
            &labels.hide,
            &labels.hide_others,
            &labels.close_window,
            &labels.undo,
            &labels.redo,
            &labels.cut,
            &labels.copy,
            &labels.paste,
            &labels.select_all,
            &labels.fullscreen,
            &labels.minimize,
            &labels.maximize,
        ] {
            assert_eq!(predefined, &None);
        }
    }

    #[test]
    fn simplified_chinese_names_every_item_and_keeps_the_app_name() {
        let labels = menu_labels(UiLanguage::SimplifiedChinese, "Mallard");
        assert_eq!(labels.about.as_deref(), Some("关于 Mallard"));
        assert_eq!(labels.hide.as_deref(), Some("隐藏 Mallard"));
        assert_eq!(labels.quit, "退出 Mallard");
        assert_eq!(labels.copy.as_deref(), Some("拷贝"));
        for predefined in [
            &labels.about,
            &labels.services,
            &labels.hide,
            &labels.hide_others,
            &labels.close_window,
            &labels.undo,
            &labels.redo,
            &labels.cut,
            &labels.copy,
            &labels.paste,
            &labels.select_all,
            &labels.fullscreen,
            &labels.minimize,
            &labels.maximize,
        ] {
            assert!(predefined.is_some());
        }
    }

    #[test]
    fn the_frontend_language_keys_parse() {
        assert_eq!(
            serde_json::from_str::<UiLanguage>("\"en\"").unwrap(),
            UiLanguage::English
        );
        assert_eq!(
            serde_json::from_str::<UiLanguage>("\"zh-CN\"").unwrap(),
            UiLanguage::SimplifiedChinese
        );
        assert!(serde_json::from_str::<UiLanguage>("\"zh-TW\"").is_err());
    }
}
