/**
 * Chinese for backend labels keyed by codes the backend sends since D7. See
 * `backend.ts`. The backend's English stays beside each code (the CLI and
 * the activity log print it); this file only holds the Chinese:
 * `discovery.…` (storage discovery facts, marker details, issues, cautions,
 * scope, and the descriptor's discovery words by provider),
 * `sessionCount.<explanation_code>`, `sessionWarning.<code>`,
 * `syncDetail.<status_detail_code | reason_code>`,
 * `readiness.<issue_id>…`, `afterPull.<message_code>…`,
 * `maintenanceResult.<summary_code>`, `catalogProgress.<stage>`,
 * `catalogOutcome.<message_code>`. Names, paths and counts arrive as
 * `{{values}}`; `backendText` does not space them, so the spacing between
 * Han and a Latin value is written here.
 */
export const zhBackendCodes = {
  discovery: {
    /** A candidate's fact chips: `fact.<key>` is the label. */
    fact: {
      jurisdiction: "管辖区",
      created: "创建于",
      access: "访问权限",
      location: "位置",
    },
    jurisdiction: {
      default: "默认",
      eu: "EU",
      us: "US",
      fedramp: "FedRAMP",
    },
    /** Dropbox's own access tags. */
    access: {
      owner: "所有者",
      editor: "可编辑",
      viewer: "可查看",
      viewer_no_comment: "可查看（不可评论）",
      unknown: "未知",
    },
    location: {
      in_dropbox: "在 Dropbox 中",
      not_mounted: "未挂载",
    },
    /** After the marker's verdict: "Mallard 存储 · 布局 1". */
    markerDetail: {
      layout: "布局 {{version}}",
      timed_out: "超时",
      read_failed: "读取失败",
      access_denied: "访问被拒绝",
      could_not_read: "无法读取标记",
      scan_timed_out: "扫描超时",
    },
    issue: {
      unreadable_buckets: "{{count}} 个存储桶无法检查。",
      listing_cut: "未能列出所有存储桶。",
      folders_partial: "部分文件夹无法列出或检查。请重新扫描。",
    },
    caution: {
      read_only: "只读权限无法上传。",
    },
    scope: {
      dropbox_shared_folders: "可访问的 Dropbox 共享文件夹",
    },
    /** The descriptor's discovery words, by provider key and field. */
    r2: {
      unit: "存储桶",
      unit_plural: "存储桶",
      place: "你的 Cloudflare 账户",
      find_label: "在我的账户中查找",
      access_hint: "使用你的 Wrangler 登录。只读：列出存储桶，并读取每个存储桶中的一个标记文件。",
      scanning_hint: "正在查看你的 Cloudflare 账户…",
      prepare_hint: "Cloudflare 会签发仅限此存储桶的访问密钥。",
    },
    dropbox: {
      unit: "共享文件夹",
      unit_plural: "共享文件夹",
      place: "你的 Dropbox",
      find_label: "在我的 Dropbox 中查找",
      access_hint: "查找会列出可访问的文件夹并读取其中的 Mallard 标记，不做任何更改。",
      scanning_hint: "请在浏览器中批准访问，随后 Mallard 会查看你的共享文件夹…",
      prepare_hint: "这是 Mallard 保留的连接，会自动续期。",
    },
  },
  /** Why a session count is not exact, by `explanation_code`. */
  sessionCount: {
    codex_records_unindexed: "部分 Codex 记录无法安全地编入索引。",
    codex_tool_activity_incomplete: "持久化的 rollout 不完整，可能缺少部分 Codex 工具活动。",
    codex_phases_inferred: "rollout 没有记录权威的助手阶段，位置是根据前后的工具活动推断的。",
    codex_checkpoints_outside_transcript: "Git 检查点由 Mallard 在 Codex 对话记录之外关联。",
    claude_records_unindexed: "部分 Claude 记录无法安全地编入索引。",
    claude_tool_activity_incomplete: "持久化的对话记录不完整或格式有误，可能缺少部分 Claude 工具活动。",
    claude_end_turn_missing: "助手消息缺少权威的回合结束标记。",
    claude_checkpoints_outside_transcript: "Git 检查点由 Mallard 在 Claude 对话记录之外关联。",
    checkpoints_not_git_worktree: "此项目不是 Git 工作树，因此没有 Git 检查点。",
    checkpoints_inferred: "检查点是根据所记录分支上的第一个后续提交推断的。",
    tokens_may_be_incomplete: "对话记录中可能还有更多 Token 用量。",
    tokens_unavailable: "此对话记录没有提供可靠的 Token 总数。",
  },
  /** A session page's notes, by warning `code`. Several codes share one
   *  English message; each has its Chinese here. */
  sessionWarning: {
    timeline_event_limit: "此会话的事件多于 Mallard 能显示的数量，其余事件未显示。",
    message_preview_truncated: "超过 1 MiB 的消息在此处会被截断，会话文件中保留完整内容。",
    affected_path_limit: "某个工具调用更改的文件多于 Mallard 能列出的数量。",
    orphan_tool_result: "有工具结果找不到对应的调用，未显示。",
    legacy_message_phase_inferred: "此会话早于 Codex 的回复标记，因此 Mallard 推测了哪些是最终回复、哪些是步骤。",
    message_phase_inferred: "部分回复没有结束标记，因此 Mallard 推测了哪些是最终回复、哪些是步骤。",
    incomplete_tool_call: "标记为未完成的工具调用从未记录结果，很可能是被中断了。",
    malformed_canonical_item: "会话文件中的部分条目无法读取，未显示。",
    malformed_response_item: "会话文件中的部分条目无法读取，未显示。",
    malformed_event: "会话文件中的部分条目无法读取，未显示。",
    malformed_assistant_record: "会话文件中的部分条目无法读取，未显示。",
    malformed_assistant_content: "会话文件中的部分条目无法读取，未显示。",
    unknown_canonical_item: "此会话包含当前的 Mallard 无法识别的条目，未显示。",
    unknown_response_item: "此会话包含当前的 Mallard 无法识别的条目，未显示。",
    unknown_tool_event: "此会话包含当前的 Mallard 无法识别的条目，未显示。",
    unknown_tool_block: "此会话包含当前的 Mallard 无法识别的条目，未显示。",
    record_limit: "会话文件超出了 Mallard 能读取的长度，其余部分未显示。",
    oversized_record: "会话文件中有一条记录过大，无法读取，未显示。",
    malformed_record: "会话文件中的部分行无法读取，未显示。",
    tool_result_missing_id: "部分工具调用无法与其结果对应。",
    tool_use_missing_id: "部分工具调用无法与其结果对应。",
    timeline_cache_ignored: "保存的时间线缓存已过时或无法读取，Mallard 已重建本地时间线缓存。",
    timeline_cache_write_failed: "Mallard 无法更新本地时间线缓存。",
  },
  /** A sync row's `status_detail_code` and a decision's `reason_code`: the
   *  sentences the service words itself. `{{value}}` is a provider's name or
   *  key, or an English error after the Chinese lead-in. */
  syncDetail: {
    capture_unreadable: "本次扫描无法安全读取此会话。",
    capture_size_limit: "超出大小限制。",
    capture_hard_linked: "会话文件是硬链接，无法安全同步。",
    capture_not_regular: "会话路径不是普通文件，无法安全同步。",
    capture_unavailable: "此会话无法安全捕获。",
    view_only_connect_profile: "仅可查看——连接 {{value}} 配置后才能同步此会话。",
    view_only_no_provider: "仅可查看——当前的 Mallard 不支持“{{value}}”提供方。",
    children_unverified: "无法为上传验证子会话。",
    local_session_unreadable: "无法读取此会话的本地记录以供检查：{{value}}",
    keep_both_restores_cloud_copy: "{{value}} 选择“两者都保留”会恢复云端副本，并保留本地文件。",
    pull_keep_both_to_restore: "{{value}} 请以“两者都保留”下载，以恢复云端副本并保留本地文件。",
    operation_pending: "此会话还有一个未完成的 {{value}} 同步操作。请先完成或恢复它。",
    detached_requires_pull: "此会话的本地映射已被替换。请下载以重新建立。",
    export_settings_changed: "导出设置已更改",
    local_metadata_changed: "会话元数据已更改。上传即可更新存储。",
    evidence_unproven: "无法安全确认会话状态。",
    cloud_draft_synced: "已同步到 Cloud 草稿。",
    cloud_draft_local_only: "不在 Cloud 草稿中。上传即可保存到那里。",
    cloud_draft_local_ahead: "有更改待上传。上传即可更新 Cloud 草稿。",
    cloud_draft_storage_only: "在 Cloud 草稿中。下载即可存到本机。",
    cloud_draft_storage_ahead: "Cloud 草稿中有更改待下载。",
  },
  /** A Pull's readiness issues, by `issue_id` (`provider-home-<provider>`
   *  as `provider-home`); `restore-project-data`'s detail by severity. */
  readiness: {
    "restore-files": {
      title: "下载尚未应用",
      detail: "打开下载检查，在这台电脑上应用你需要的更改。",
    },
    "conversation-conflicts": {
      title: "{{count}} 个会话冲突仍需手动解决",
      detail: "其他所选更改已恢复。存在冲突的本地会话保持不变。",
    },
    "restore-project-data": {
      title: "{{count}} 项项目数据更改未应用",
      detail: {
        error: "请查看技术结果，修复报告的问题，然后重新下载。",
        warning: "在这台电脑上应用此更改之前，请单独检查。",
      },
    },
    "provider-home": {
      title: "{{provider}} 主目录未映射",
      detail: "恢复提供方状态前，请选择一个现有的 {{provider}} 主目录。",
    },
  },
  /** A finished Pull's offers, by `message_code`. */
  afterPull: {
    restart_app: {
      label: "重启 {{app}}",
      message: "{{app}} 可能需要重启才能刷新会话列表。",
    },
    restart_app_with_home: {
      label: "重启 {{app}}",
      message: "{{app}} 可能需要重启才能刷新会话列表。Mallard 会使用此项目的 Codex 主目录（{{path}}）重新启动它。",
    },
    restart_app_manually: {
      label: "重启 {{app}}",
      message: "如果看不到已同步的会话，请手动重启 {{app}}。",
    },
  },
  /** What a finished maintenance action did, by `summary_code`. */
  maintenanceResult: {
    close_requested: "已请求关闭。Codex 关闭后，请再次尝试删除。",
    app_already_closed: "{{app}} 已关闭，下次打开时会刷新会话列表。",
    app_restarted: "{{app}} 已重启。",
    app_restarted_with_home: "{{app}} 已使用此项目的 Codex 主目录（{{path}}）重启。",
  },
  /** The Library's Push and Pull progress line, by `stage`. */
  catalogProgress: {
    publishing: "正在发布已检查的项…",
    pushing: "正在上传 {{name}}…",
    push_finished: "上传完成",
    pulling: "正在下载 {{name}}…",
  },
  /** The Library's fixed result messages: an action result's
   *  `message_code`, and a finished removal. */
  catalogOutcome: {
    skipped: "已跳过，本地内容保持不变",
    forked: "已从“{{name}}”复刻；上传之前仅保存在本地",
    kept_as: "你本地的内容已另存为“{{name}}”",
    removed: "已从此代理主目录移除 {{name}}。存储中的副本会保留。",
  },
} as const;
