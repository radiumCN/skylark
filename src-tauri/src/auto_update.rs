use std::time::Duration;
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// Show a native desktop notification. These fire from background tasks that may run
/// while the window is hidden/minimized, so a system notification is the only way the
/// user sees them. Failures (e.g. OS permission denied) are ignored — a missing toast
/// must never break the update flow.
fn notify(app_handle: &tauri::AppHandle, title: &str, body: &str) {
    let _ = app_handle
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// Compare two dotted numeric versions and report whether `latest` is strictly newer
/// than `current`. A leading `v` and any pre-release suffix after `-` are ignored, and
/// non-numeric / missing components are treated as 0 so a malformed value can never
/// spuriously trigger an "update available". This replaces a plain `!=` comparison,
/// which wrongly fired when the local version merely differed (e.g. equal-but-tagged,
/// or a manually-installed newer build).
fn is_newer_version(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim().trim_start_matches('v')
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|s| s.trim().parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(latest), parts(current));
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// Background task: check for sing-box updates on startup and periodically.
/// Emits "singbox-update-available" { version, download_url, release_notes } when a new version is found.
pub async fn start_auto_update_checker(app_handle: tauri::AppHandle, interval_hours: u32) {
    // First check: 30 seconds after launch (let app settle)
    tokio::time::sleep(Duration::from_secs(30)).await;
    check_and_emit(&app_handle).await;

    if interval_hours == 0 {
        return;
    }

    let interval = Duration::from_secs(interval_hours as u64 * 3600);
    loop {
        tokio::time::sleep(interval).await;
        check_and_emit(&app_handle).await;
    }
}

// ─── Subscription Auto-Updater ───────────────────────────────────────

/// Background task: periodically refresh subscriptions that have auto_update enabled.
/// Checks every 30 minutes; updates any subscription whose last_update is older than its interval.
pub async fn start_subscription_auto_updater(app_handle: tauri::AppHandle) {
    // Wait for app to settle before first check
    tokio::time::sleep(Duration::from_secs(60)).await;

    loop {
        let state = app_handle.state::<crate::commands::AppState>();

        let to_update: Vec<(String, String, String)> = {
            let subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
            let now = chrono::Utc::now();
            subs.iter()
                .filter(|s| s.auto_update && s.update_interval > 0)
                .filter(|s| match s.last_update {
                    None => true,
                    Some(last) => (now - last).num_hours() >= s.update_interval as i64,
                })
                .map(|s| (s.id.clone(), s.url.clone(), s.name.clone()))
                .collect()
        };

        for (id, url, name) in to_update {
            match do_update_subscription(&app_handle, &id, &url).await {
                Ok(count) => {
                    log::info!("订阅自动更新成功: {}", id);
                    notify(
                        &app_handle,
                        "订阅已更新",
                        &format!("{} 更新成功，共 {} 个节点", name, count),
                    );
                }
                Err(e) => {
                    log::warn!("订阅自动更新失败 [{}]: {}", id, e);
                    notify(&app_handle, "订阅更新失败", &format!("{}：{}", name, e));
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    }
}

async fn do_update_subscription(
    app_handle: &tauri::AppHandle,
    id: &str,
    url: &str,
) -> anyhow::Result<usize> {
    // Reuse the shared fetch (direct first, then retry through the local proxy for
    // GFW-blocked airport hosts) so the background updater behaves like the manual one.
    let state = app_handle.state::<crate::commands::AppState>();
    let proxy_port = crate::commands::active_proxy_port(&state);
    let (content, userinfo) = crate::commands::fetch_url(url, proxy_port).await?;

    crate::config::save_subscription_content(id, &content)?;
    let sub_type = crate::subscription::detect_sub_type(&content, url);
    let (nodes, outbounds) = crate::subscription::parse_subscription(&content, id)?;
    // Apply the subscription's stored name filters / region grouping.
    let (include, exclude, group_by_region) = {
        let subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        subs.iter()
            .find(|s| s.id == id)
            .map(|s| (s.include.clone(), s.exclude.clone(), s.group_by_region))
            .unwrap_or((None, None, false))
    };
    let (nodes, outbounds) = crate::subscription::apply_node_filters(
        nodes,
        outbounds,
        include.as_deref(),
        exclude.as_deref(),
        group_by_region,
    );
    let node_count = nodes.len();

    {
        let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sub) = subs.iter_mut().find(|s| s.id == id) {
            sub.sub_type = sub_type;
            sub.node_count = nodes.len();
            sub.last_update = Some(chrono::Utc::now());
            // Preserve known quota when a refresh omits the header.
            if userinfo.upload.is_some() { sub.upload = userinfo.upload; }
            if userinfo.download.is_some() { sub.download = userinfo.download; }
            if userinfo.total.is_some() { sub.total = userinfo.total; }
            if userinfo.expire.is_some() { sub.expire = userinfo.expire; }
        }
        crate::config::save_subscriptions(&subs)?;
    }
    {
        let mut all_nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        all_nodes.retain(|n| n.subscription_id.as_deref() != Some(id));
        all_nodes.extend(nodes);
        crate::config::save_nodes(&all_nodes)?;
    }
    {
        let mut all_outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner());
        let new_tags: std::collections::HashSet<String> = outbounds.iter()
            .filter_map(|ob| ob["tag"].as_str().map(|s| s.to_string()))
            .collect();
        all_outbounds.retain(|ob| {
            ob["tag"].as_str()
                .map(|t| !new_tags.contains(t))
                .unwrap_or(true)
        });
        all_outbounds.extend(outbounds);
        crate::config::save_outbounds(&all_outbounds)?;
    }

    Ok(node_count)
}

// ─── App Self-Update Checker ─────────────────────────────────────────

/// Background task: check for app updates once at startup (after a short delay).
/// Emits "app-update-available" { version, download_url, release_notes, is_prerelease }
/// when the latest release version differs from the running app version.
pub async fn start_app_update_checker(app_handle: tauri::AppHandle) {
    // Wait 45 seconds after launch so the window is fully up before showing anything
    tokio::time::sleep(std::time::Duration::from_secs(45)).await;

    let channel = {
        let cfg = crate::config::load_app_config();
        cfg.update_channel.clone()
    };

    match crate::updater::fetch_app_release(&channel, false).await {
        Ok(release) => {
            // Source the running version from the bundle metadata (tauri.conf.json →
            // package.json) so package.json is the single version to bump per release,
            // rather than also having to keep Cargo.toml's CARGO_PKG_VERSION in sync.
            let current = app_handle.package_info().version.to_string();
            let latest = release.version.trim_start_matches('v');
            if is_newer_version(latest, &current) {
                let _ = app_handle.emit("app-update-available", serde_json::json!({
                    "version": release.version,
                    "download_url": release.download_url,
                    "release_notes": release.release_notes,
                    "published_at": release.published_at,
                    "is_prerelease": release.is_prerelease,
                    "current_version": current,
                }));
                notify(
                    &app_handle,
                    "发现新版本",
                    &format!("Skylark {} 可供更新", release.version),
                );
            }
        }
        Err(e) => {
            log::debug!("应用更新检查失败: {}", e);
        }
    }
}

// ─── sing-box Binary Updater ─────────────────────────────────────────

async fn check_and_emit(app_handle: &tauri::AppHandle) {
    match crate::updater::fetch_latest_release(false).await {
        Ok(release) => {
            let installed = crate::updater::get_installed_version().await;
            let installed_ver = installed
                .as_deref()
                .and_then(|s| s.split_whitespace().last())
                .unwrap_or("");
            let latest_ver = release.version.trim_start_matches('v');

            if !installed_ver.is_empty() && is_newer_version(latest_ver, installed_ver) {
                let _ = app_handle.emit("singbox-update-available", serde_json::json!({
                    "version": release.version,
                    "download_url": release.download_url,
                    "release_notes": release.release_notes,
                    "installed_version": installed_ver,
                }));
                notify(
                    app_handle,
                    "内核可更新",
                    &format!("sing-box 内核 {} 可供更新", release.version),
                );
            } else if installed_ver.is_empty() {
                // Not installed at all — notify frontend
                let _ = app_handle.emit("singbox-not-installed", serde_json::json!({
                    "latest_version": release.version,
                    "download_url": release.download_url,
                }));
            }
        }
        Err(e) => {
            log::warn!("自动更新检查失败: {}", e);
        }
    }
}
