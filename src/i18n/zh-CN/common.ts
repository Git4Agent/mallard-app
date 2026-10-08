import type { TranslatedCatalog } from "../catalog";
import type en from "../en/common";

export default {
  push: "上传",
  pull: "下载",
  cancel: "取消",
  close: "关闭",
  save: "保存",
  delete: "删除",
  remove: "移除",
  open: "打开",
  back: "返回",
  tryAgain: "重试",
  refresh: "刷新",
  copy: "复制",
  copied: "已复制",
  ok: "好",
  settings: "设置",
  loading: "正在载入…",
  saving: "正在保存…",
  saved: "已保存",
  notSaved: "未保存",
  version: "Version {{version}}",
  pushFinished: "已完成。请重新比较。",
} satisfies TranslatedCatalog<typeof en>;
