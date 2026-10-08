import type { TranslatedCatalog } from "../catalog";
import type en from "../en/format";

export default {
  unknown: "未知",
  justNow: "刚刚",
  minutesAgo: "{{value}}分钟前",
  hoursAgo: "{{value}}小时前",
  dateAtTime: "{{date}} {{time}}",
  seconds: "{{value}}秒",
  minutes: "{{value}}分钟",
  hours: "{{value}}小时",
  hoursMinutes: "{{hours}}小时{{minutes}}分钟",
  days: "{{value}}天",
  daysHours: "{{days}}天{{hours}}小时",
} satisfies TranslatedCatalog<typeof en>;
