import type { TranslatedCatalog } from "../catalog";
import type en from "../en/notice";

export default {
  severity: {
    danger: "错误",
    warning: "警告",
    info: "提示",
    success: "完成",
  },
  dismiss: "关闭",
  dismissWhat: "关闭：{{what}}",
  spoken: {
    stop: "{{part}}。",
  },
  zone: {
    count: "{{count}} {{subject}}",
    all: "全部 {{count}} {{subject}}",
  },
  field: {
    spoken: "错误：",
    dismiss: "错误。{{text}}",
  },
} satisfies TranslatedCatalog<typeof en>;
