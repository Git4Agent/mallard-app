/**
 * Chinese for backend labels keyed by identifiers the backend already sends.
 * See `backend.ts`. Every key is built from the backend's own ids:
 * `storageProvider.<provider key>…`, `storageSummary.<field_id>`,
 * `resourceKind.<kind key>`, `applyPolicy.<policy key>`,
 * `capability.<presentation icon>…`, `agentProfile.<provider>`,
 * `maintenance.<action_id>…`, `sessionAction.<action_id>`,
 * `blocker.<blocker>`. Persisted names (default storage names, name
 * templates) are user data once saved and never appear here.
 */
export const zhBackendLabels = {
  /** `list_storage_providers` descriptors, by provider key. Discovery
   *  strings are keyed with the codes in `backend-codes.ts`. */
  storageProvider: {
    local: {
      displayName: "本地文件夹",
      description: "本地文件夹或已挂载的磁盘。",
      field: {
        directory: { label: "位置路径", placeholder: "例如：/Volumes/backup/mallard-storage" },
      },
    },
    r2: {
      displayName: "Cloudflare R2",
      description: "将快照存放在你的 R2 存储桶中。",
      field: {
        api_url: {
          label: "S3 API URL",
          placeholder: "例如：https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent",
          help: "如何找到 Cloudflare R2 的 S3 API URL",
        },
        bucket: {
          label: "存储桶名称",
          placeholder: "例如：codex-sync-backups",
          help: "如何创建 Cloudflare R2 存储桶",
        },
        account_id: {
          label: "账户 ID",
          placeholder: "例如：023e105f4ecef8ad9ca31a8372d0c353",
          help: "在哪里找到 Cloudflare 账户 ID",
        },
        access_key_id: {
          label: "访问密钥 ID",
          placeholder: "例如：4a2b…91ef",
          help: "如何创建 R2 访问密钥 ID",
        },
      },
      secret: {
        secret_access_key: {
          label: "私有访问密钥",
          placeholder: "例如：a1B2…x9Y0",
          help: "如何创建 R2 私有访问密钥",
        },
      },
      manualSetupHint: "在下方字段中填写已有的 R2 连接。",
      doors: {
        createHint: "在你的 Cloudflare 账户中新建存储桶。",
        existingHint: "Mallard 已在使用的存储桶。",
        existingHelpLabel: "设置团队访问",
      },
    },
    dropbox: {
      displayName: "Dropbox",
      description: "Dropbox 文件夹。单个对象最大 150 MiB。",
      field: {
        namespace_id: {
          label: "命名空间 ID",
          placeholder: "共享或团队命名空间 ID",
          help: "每台电脑请使用相同的命名空间 ID。设置后无法更改。",
        },
      },
      secret: {
        access_token: {
          label: "访问令牌",
          placeholder: "关联到用户的 Dropbox 访问令牌",
          help: "手动设置时，请授予 account_info.read、files.metadata.read、files.content.read 和 files.content.write 权限。令牌过期后请替换。通过浏览器设置会自动续期访问权限。",
        },
        refresh_token: {
          label: "刷新令牌",
          placeholder: "Dropbox 刷新令牌",
          help: "通过浏览器设置时会保存此令牌，用于自动续期访问权限。点“显示”可查看它的值，也可以填入为此 Mallard 应用和 Dropbox 账户签发的新令牌。",
        },
      },
      manualSetupHint: "填写已有的共享或团队命名空间，以及关联到用户的令牌。Mallard 会把数据存放在该命名空间的 .mallard 中。",
      doors: {
        createHint: "在你的 Dropbox 中新建共享文件夹。",
        existingHint: "Mallard 已在使用的共享文件夹。",
      },
    },
    feishu: {
      displayName: "飞书",
      description: "飞书 Drive 文件夹。单个对象最大 100 MiB，更大的文件分块存储。",
      field: {
        folder_token: {
          label: "文件夹令牌",
          placeholder: "该 Drive 文件夹的令牌",
          help: "此存储所在的 Drive 文件夹。设置后不可更改；请在飞书中把它共享给每位要连接的用户。",
        },
      },
      doors: {
        createHint: "在“我的空间”中新建文件夹。",
        existingHint: "在飞书中共享给你的文件夹。",
      },
    },
    managed_cloud: {
      displayName: "Mallard Cloud",
      description: "私有快照，可与团队共享并在网页上查看。",
      secret: {
        account_token: {
          label: "登录令牌",
          placeholder: "由浏览器登录签发",
          help: "登录时签发，有效期 90 天。重新连接即可续期。",
        },
      },
    },
  },
  /** `StorageSummary.summary[].label`, by `field_id`: one word per id across
   *  every provider. The values stay as the backend wrote them. */
  storageSummary: {
    directory: "文件夹",
    bucket: "存储桶",
    account_id: "账户 ID",
    endpoint: "S3 端点",
    access_key_id: "访问密钥 ID",
    folder_name: "文件夹",
    namespace_id: "命名空间 ID",
    folder_token: "文件夹令牌",
    account_name: "账户",
    reauthorize_by: "重新登录期限",
    account_login: "账户",
    service_origin: "服务",
  },
  /** `InventoryResource.kind_label`, by `kind.key`. */
  resourceKind: {
    conversation: "会话",
    provider_state: "提供方状态",
    project_content_file: "项目文件",
    project_content_directory: "项目文件夹",
    requirement: "依赖",
  },
  /** `InventoryResource.apply_policy_label`, by `apply_policy.key`. */
  applyPolicy: {
    safe_file: "安全时应用",
    merge: "下载时合并",
    explicit_review: "检查后应用",
    manual_only: "仅手动应用",
    never: "从不应用",
  },
  /** Library capability labels, by `presentation.icon`. Skill stays English
   *  and singular: Skill · Claude, 插件 · Codex, 宠物 v2 · Codex. */
  capability: {
    skill: { label: "Skill", singular: "Skill", kindLabel: "Skill", formatLabel: "Agent Skill" },
    plugin: { label: "插件", singular: "插件", kindLabel: "插件", formatLabel: "插件安装意向" },
    pet: { label: "宠物 v1", singular: "宠物 v1", kindLabel: "宠物", formatLabel: "Codex 宠物 v1" },
    "pet-v2": { label: "宠物 v2", singular: "宠物 v2", kindLabel: "宠物", formatLabel: "Codex 宠物 v2" },
  },
  /** `ProviderDescriptor.profile_label`, by provider key. */
  agentProfile: {
    codex: ".codex 配置",
    claude: ".claude 配置",
  },
  /** `MaintenanceActionDescriptor` label and description, by `action_id`. */
  maintenance: {
    "codex.app.restart": {
      label: "重启 Codex 应用",
      description: "重新启动 Codex 桌面应用，让恢复的会话显示出来。",
    },
    "codex.app.close": {
      label: "关闭 Codex",
      description: "关闭 Codex 可能会中断正在进行的对话。",
    },
  },
  /** `SessionActionDescriptor.label` by `action_id`, and its
   *  `disabled_reason` beside it as `<action_id>.disabledReason`. */
  sessionAction: {
    "codex.open-in-app": "在 Codex 中打开",
    "codex.open-in-app.disabledReason": "此会话不在本机上。",
    "codex.open-in-terminal": "在终端中打开",
    "codex.open-in-terminal.disabledReason": "此会话不在本机上。",
    "claude.open-in-app": "在 Claude 中打开",
    "claude.open-in-app.disabledReason": "此会话不在本机上。",
    "claude.open-in-terminal": "在终端中打开",
    "claude.open-in-terminal.disabledReason": "此会话不在本机上。",
  },
  /**
   * A blocked conversation decision's `blocked_reason`, by `blocker`.
   * `codec_unsupported`, `missing_binding` and `observation_failed` are left
   * out on purpose: the service replaces their text with a formatted
   * sentence (the capture failure, "View only — …", the read error) under
   * the same blocker, so the code cannot tell which sentence came. They show
   * the backend's English until the backend sends a code of their own.
   */
  blocker: {
    ambiguous_binding: "有多个本地与云端的会话绑定都相符。",
    busy: "另一个同步操作仍在处理此会话，请等它完成。",
    detached_requires_pull: "此会话的本地映射已被替换。请下载以重新建立。",
    unknown_evidence: "无法安全地确认此会话在本地和存储中的状态。",
    redaction_ruleset_mismatch: "此会话存储时所用的密钥隐藏规则与当前的 Mallard 不一致。请先在所有电脑上更新 Mallard，再同步它。",
    path_map_codec_mismatch: "此会话存储时所用的路径编码与当前的 Mallard 不一致。请先在所有电脑上更新 Mallard，再同步它。",
  },
} as const;
