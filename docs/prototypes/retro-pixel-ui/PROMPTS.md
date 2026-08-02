# Mallard Retro Pixel UI Prototype Set

Generated with the built-in image generation tool from two user-provided
references:

- Mallard's current desktop information architecture and screen layout.
- The desired dark retro-pixel visual direction.

## Shared prompt

Create a high-fidelity Mallard desktop UI prototype in a 16:9 landscape
window. Preserve the current single-sidebar information architecture:
Mallard brand, Projects/Storages/Logs navigation, project and storage lists,
and the selected workflow in the main workspace.

Use a relaxed 1990s desktop-utility aesthetic with polished 16-bit pixel art:

- deep, soft-colored backgrounds instead of pure black;
- warm cream primary text;
- stepped square borders and controls;
- readable pixel-mono body text with a chunkier display face for headings;
- subtle scanlines and sparse pixel dithering;
- varied filled pixel icons on small color tiles;
- an 8px spacing rhythm and accessible contrast;
- distinct semantic colors for sync, Git, warnings, healthy state, focus, and
  primary actions.

Avoid aggressive cyberpunk glow, extra navigation rails, unrelated dashboard
cards, glass effects, rounded cards, emoji, device frames, perspective,
photorealism, credentials, personal absolute paths, and illegible filler text.

## Screen prompts

1. **Project Git history — Blueberry**
   Preserve the current project page with the linked storage row, Pull/Push
   controls, branch context, commit timeline, and expanded conversation tree.
   Use aqua, coral, butter, periwinkle, mint, and soft pink accents.

2. **Expanded conversation — Plum Soda**
   Expand one conversation inline under a commit. Show a compact chat preview,
   thread metadata, and Open in Codex / Copy ID / Close controls.

3. **Storages overview — Teal Dusk**
   Show one selected local storage, its repositories, health summary, project
   and resource counts, and Open folder / Edit / Add storage actions.

4. **New R2 storage — Sunset Plum**
   Show Local folder and Cloudflare R2 options, labeled connection fields,
   a three-step setup guide, local-credential notice, and test/save actions.

5. **Add project setup — Forest Terminal**
   Show Project, Agent, Storage, Repository, and Sync contents sections with a
   Git-matched repository recommendation and explicit finish action.

6. **Push review: sessions — Berry Cream**
   Show the four review steps, selected Git/Codex/Claude resources,
   destination readiness, recommended selection, and local-data safety note.

7. **Push review: project files — Cobalt Orange**
   Show a non-Git file list, executable and credential-shaped warnings,
   explicit exclusion/acknowledgement controls, and disabled Continue state.

8. **Pull review: conflict — Teal Raspberry**
   Show safe/conflict counts, session actions, local-versus-storage comparison,
   keep-both recommendation, and an explicit plan-apply action.

9. **Local activity logs — Amber Violet**
   Show sanitized event history, filters, local log-storage statistics,
   retention controls, cleanup, and save actions.

10. **Repository identity dialog — Pastel CRT**
    Overlay a repository chooser on the project-history screen. Show the
    recommended portable history, an archive, a separate-repository option,
    and Connect and review Pull.
