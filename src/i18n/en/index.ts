import common from "./common";
import format from "./format";
import shell from "./shell";
import settings from "./settings";
import syncReview from "./syncReview";
import notice from "./notice";
import logs from "./logs";
import projects from "./projects";
import session from "./session";
import reviews from "./reviews";
import storage from "./storage";
import projectSetup from "./projectSetup";
import cloud from "./cloud";
import projectPage from "./projectPage";
import library from "./library";

/** The source catalogs, one namespace per area. */
export const en = {
  common,
  format,
  shell,
  settings,
  syncReview,
  notice,
  logs,
  projects,
  session,
  reviews,
  storage,
  projectSetup,
  cloud,
  projectPage,
  library,
} as const;
