import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import "../src/App.css";
import "./message-model-directions-mockup.css";

/*
 * Model and thinking on a session's messages: eight ways, side by side
 * (2026-10-07). Every frame holds the same five Claude messages: two turns,
 * the second answered twice at different effort. The owner picked D on
 * 2026-10-07; every frame is a drawing.
 *   ?theme=light|dark
 */

type Role = "prompt" | "response";
interface Msg {
  id: string;
  role: Role;
  time: string;
  duration?: string;
  calls?: string;
  model: string;
  effort: string;
  body: string;
}

const OPUS = "claude-opus-4-6";
const SONNET = "claude-sonnet-4-6";
const MSGS: Msg[] = [
  { id: "u1", role: "prompt", time: "10:02", duration: "24s", calls: "2 calls", model: OPUS, effort: "Low", body: "Check upload failures." },
  { id: "a1", role: "response", time: "10:02", model: OPUS, effort: "Low", body: "A timeout can occur after the file is stored." },
  { id: "u2", role: "prompt", time: "10:04", duration: "1m 12s", calls: "3 calls", model: SONNET, effort: "Mixed", body: "Add a test for a lost response." },
  { id: "a2", role: "response", time: "10:04", model: SONNET, effort: "Low", body: "Added." },
  { id: "a3", role: "response", time: "10:05", model: SONNET, effort: "High", body: "All tests passed." },
];
const word = (m: Msg) => `${m.model} · ${m.effort}`;

/* The band word of the first round (A, F), drawn: `model · level` at the band's end. */
function Stamp({ m }: { m: Msg }) {
  return <button type="button" className="dir-end dir-word dir-disclosure" aria-haspopup="dialog">{word(m)}</button>;
}

function Card({ m, head, between, foot, tileDot, tip, cardClass }: {
  m: Msg;
  head?: ReactNode;
  between?: ReactNode;
  foot?: ReactNode;
  tileDot?: boolean;
  tip?: string;
  cardClass?: string;
}) {
  return (
    <li>
      <article className={`session-detail-event event-${m.role}`}>
        <span className={`session-detail-event-node dir-tile${tileDot ? " dir-tile-dot" : ""}`} title={m.role === "prompt" ? "You" : "Claude"}>
          {m.role === "prompt" ? <Icon name="user" size={12} /> : <ProviderMark provider="claude" displayName="Claude" size={12} />}
          {tip && <span className="dir-tip">{tip}</span>}
        </span>
        <div className={`session-detail-card session-detail-card-${m.role === "prompt" ? "prompt" : "reply"}${cardClass ? ` ${cardClass}` : ""}`}>
          <div className="session-detail-card-head">
            <time>{m.time}</time>
            {m.duration && <span>{m.duration}</span>}
            {m.calls && <span>{m.calls}</span>}
            {head}
          </div>
          {between}
          <div className="session-detail-event-copy markdown-copy"><p>{m.body}</p></div>
          {foot}
        </div>
      </article>
    </li>
  );
}

function Strip({ model, effort }: { model: string; effort: string }) {
  return (
    <div className="session-detail-strip dir-strip">
      <div className="session-detail-chips">
        <span className="session-detail-chip"><Icon name="git-branch" size={12} />main</span>
        <span className="session-detail-chip"><Icon name="clock" size={12} />Oct 7, 2026 · 10:02</span>
        <span className="session-detail-chip"><Icon name="token" size={12} /><b>4.8M</b></span>
        <button type="button" className="session-detail-chip dir-model-chip" aria-haspopup="dialog" aria-expanded="false">
          <ProviderMark provider="claude" displayName="Claude" size={12} /><code>{model}</code> · {effort}
        </button>
      </div>
    </div>
  );
}

function Change({ m }: { m: Msg }) {
  return (
    <li className="dir-note">
      <div className="session-detail-day-divider dir-change">
        <button type="button" aria-haspopup="dialog" aria-expanded="false">{word(m)}</button>
      </div>
    </li>
  );
}

function Frame({ letter, name, seen, against, className, children }: {
  letter: string;
  name: string;
  seen: [string, string][];
  against: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={`dir${className ? ` ${className}` : ""}`} aria-label={`${letter}: ${name}`}>
      <header className="dir-head"><b>{letter}</b><h2>{name}</h2></header>
      <div className="dir-stage">{children}</div>
      <footer className="dir-foot-line">
        <span>Seen in</span>
        {seen.map(([label, href]) => <a key={href} href={href} target="_blank" rel="noopener noreferrer">{label}</a>)}
        <span className="dir-against">Against: {against}</span>
      </footer>
    </section>
  );
}

const List = ({ children }: { children: ReactNode }) => <ol className="session-detail-event-list">{children}</ol>;

function Page() {
  const [u1, a1, u2, a2, a3] = MSGS;
  const theme = new URLSearchParams(window.location.search).get("theme") === "dark" ? "dark" : "light";
  return (
    <div className="dirs-page">
      <header className="dirs-bar">
        <h1>Model and thinking on messages</h1>
        <span>eight ways · same five messages</span>
        <a href={`?theme=${theme === "dark" ? "light" : "dark"}`}>{theme === "dark" ? "Light" : "Dark"}</a>
      </header>
      <div className="dirs">
        <Frame letter="A" name="Band word, click for facts" against="Repeats on every card."
          seen={[["HuggingChat route label", "https://huggingface.co/spaces/huggingchat/chat-ui/discussions/764"], ["Carbon toggletip", "https://carbondesignsystem.com/components/toggletip/usage/"]]}>
          <List>{MSGS.map((m) => <Card key={m.id} m={m} head={<Stamp m={m} />} />)}</List>
        </Frame>

        <Frame letter="B" name="Band chips, level tinted" against="Chips read as actions; loud."
          seen={[["Material chips", "https://developer.android.com/develop/ui/compose/components/chip"], ["Primer Label", "https://primer.style/product/components/label"]]}>
          <List>{MSGS.map((m) => (
            <Card key={m.id} m={m} head={
              <span className="dir-end">
                <span className="dir-chip"><code>{m.model}</code></span>
                <span className={`dir-chip level ${m.effort.toLowerCase()}`}>{m.effort}</span>
              </span>
            } />
          ))}</List>
        </Frame>

        <Frame letter="C" name="Footer on hover" against="Hidden until hover; no touch."
          seen={[["WCAG 1.4.13", "https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html"], ["NN/g tooltips", "https://www.nngroup.com/articles/tooltip-guidelines/"]]}>
          <List>{MSGS.map((m) => (
            <Card key={m.id} m={m} cardClass={m.id === "a2" ? "is-hover" : undefined} foot={
              <div className="dir-foot"><span>{word(m)}</span><button type="button" className="btn-link">Details</button></div>
            } />
          ))}</List>
        </Frame>

        <Frame letter="D" name="Header fact, notes on change" against="Scroll back to find current."
          seen={[["ChatGPT per-message note", "https://community.openai.com/t/gpt-5-used-despite-selecting-gpt-4o-undisclosed-model-substitution-and-mislabeled-responses/1360171"], ["Claude Code transcript notice", "https://code.claude.com/docs/en/model-config"]]}>
          <Strip model={OPUS} effort="Low" />
          <List>
            <Card m={u1} /><Card m={a1} />
            <Change m={a2} />
            <Card m={u2} /><Card m={a2} />
            <Change m={a3} />
            <Card m={a3} />
          </List>
        </Frame>

        <Frame letter="E" name="Tile tooltip, dot on change" against="Tooltip on a non-button; tiny."
          seen={[["Carbon definition tooltip", "https://carbondesignsystem.com/components/tooltip/usage/"], ["Primer: no tooltip on spans", "https://primer.style/product/components/tooltip/guidelines/"]]}>
          <List>
            <Card m={u1} /><Card m={a1} />
            <Card m={u2} tileDot />
            <Card m={a2} tip={word(a2)} />
            <Card m={a3} tileDot />
          </List>
        </Frame>

        <Frame letter="F" name="Header fact, word when it differs" against="Two places to read."
          seen={[["Claude.ai model under the input", "https://support.claude.com/en/articles/8114491-get-started-with-claude"], ["ChatGPT note when it differs", "https://community.openai.com/t/gpt-5-used-despite-selecting-gpt-4o-undisclosed-model-substitution-and-mislabeled-responses/1360171"]]}>
          <Strip model={OPUS} effort="Low" />
          <List>{MSGS.map((m) => (
            <Card key={m.id} m={m} head={m.model !== OPUS ? <Stamp m={m} /> : undefined} />
          ))}</List>
        </Frame>

        <Frame letter="G" name="Band word, hover tooltip" against="Hover text; touch needs tap." className="dir-g"
          seen={[["Material rich tooltip", "https://developer.android.com/develop/ui/compose/components/tooltip"], ["HIG popovers", "https://developer.apple.com/design/human-interface-guidelines/popovers"]]}>
          <List>{MSGS.map((m) => (
            <Card key={m.id} m={m} head={
              <span className="dir-end dir-word" tabIndex={0}>
                {word(m)}
                {m.id === "a2" && (
                  <span className="dir-rich" role="tooltip">
                    <b>Model and thinking</b>
                    <span><span>Model</span><code>{m.model}</code></span>
                    <span><span>Thinking</span><span>{m.effort}</span></span>
                    <span><span>Source</span><span>Reply record</span></span>
                  </span>
                )}
              </span>
            } />
          ))}</List>
        </Frame>

        <Frame letter="H" name="Band word, opens in place" against="Cards grow when opened."
          seen={[["ARIA disclosure", "https://www.w3.org/WAI/ARIA/apg/patterns/disclosure/"], ["Primer summary disclosure", "https://primer.style/product/components/tooltip/guidelines/"]]}>
          <List>{MSGS.map((m) => (
            <Card key={m.id} m={m}
              head={
                <button type="button" className="dir-end dir-word dir-disclosure" aria-expanded={m.id === "a2"}>
                  {word(m)}<Icon name={m.id === "a2" ? "chevron-down" : "chevron-right"} size={11} />
                </button>
              }
              between={m.id === "a2" ? (
                <div className="dir-facts">
                  <span>Model <code>{m.model}</code></span>
                  <span>Thinking <code>{m.effort}</code></span>
                  <span>Source <code>Reply record</code></span>
                </div>
              ) : undefined}
            />
          ))}</List>
        </Frame>
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = new URLSearchParams(window.location.search).get("theme") === "dark" ? "dark" : "light";
createRoot(document.getElementById("root")!).render(<StrictMode><Page /></StrictMode>);
