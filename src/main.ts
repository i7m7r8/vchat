import { listen } from "@tauri-apps/api/event";
import jsQR from "jsqr";
import { api, Contact, Identity, Group, GroupMessage, Reaction, TypingStatus, CallLogEntry, FileTransfer, AppSettings, Message } from "./lib/api";

/* ══════════════════════ HELPERS ══════════════════════ */

const $ = (sel: string): HTMLElement | null => document.querySelector(sel);
const $$ = (sel: string): NodeListOf<HTMLElement> => document.querySelectorAll(sel);
const esc = (s: any) => String(s ?? "").replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c] as string));
const timeFmt = (ts: number): string => {
  const d = new Date(ts * 1000);
  const now = new Date();
  const startOfDay = (dt: Date) => new Date(dt.getFullYear(), dt.getMonth(), dt.getDate()).getTime();
  if (startOfDay(d) === startOfDay(now)) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString([], { month: "short", day: "numeric" });
  return d.toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" });
};
const dayFmt = (ts: number): string => {
  const d = new Date(ts * 1000);
  const now = new Date();
  const yd = new Date(now); yd.setDate(now.getDate() - 1);
  const startOfDay = (dt: Date) => new Date(dt.getFullYear(), dt.getMonth(), dt.getDate()).getTime();
  if (startOfDay(d) === startOfDay(now)) return "Today";
  if (startOfDay(d) === startOfDay(yd)) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", year: "numeric", month: "long", day: "numeric" });
};
const fmtBytes = (n: number): string => {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n/1024).toFixed(1)} KB`;
  if (n < 1073741824) return `${(n/1048576).toFixed(1)} MB`;
  return `${(n/1073741824).toFixed(1)} GB`;
};

const avatarColor = (name: string): string => {
  const colors = ["#6366f1","#8b5cf6","#a855f7","#ec4899","#f43f5e","#f97316","#f59e0b","#84cc16","#22c55e","#10b981","#14b8a6","#06b6d4","#0ea5e9","#3b82f6"];
  let h = 0; for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return colors[h % colors.length];
};
const initials = (name: string): string => name.split(/\s+/).map(w => w[0]).slice(0,2).join('').toUpperCase();
const shortOnion = (onion: string): string => onion && onion.length > 12 ? `${onion.slice(0,6)}…${onion.slice(-6)}` : onion;

const escSVG = esc, S = {
  icon(name: string, size = 20): string {
    const paths: Record<string, string[]> = {
      menu: ["M3 6h18M3 12h18M3 18h18"],
      search: ["M21 21l-4.34-4.34","M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z"],
      plus: ["M12 5v14M5 12h14"],
      back: ["M19 12H5","M12 19l-7-7 7-7"],
      call: ["M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7A2 2 0 0 1 22 16.92z"],
      video: ["m22 8-6 4 6 4V8Z","M2 6h14v12H2z"],
      mic: ["M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z","M19 10v2a7 7 0 0 1-14 0v-2","M12 19v3"],
      camoff: ["M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94","M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19","M1 1l22 22"],
      screen: ["M2 3h20v14H2z","M8 21h8","M12 17v4"],
      send: ["m22 2-7 20-4-9-9-4z","M22 2 11 13"],
      attach: ["m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"],
      settings: ["M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z","M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"],
      user: ["M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2","M12 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z"],
      camera: ["M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z","M12 17a4 4 0 1 0 0-8 4 4 0 0 0 0 8z"],
      qr: ["M3 7V5a2 2 0 0 1 2-2h2","M17 3h2a2 2 0 0 1 2 2v2","M21 17v2a2 2 0 0 1-2 2h-2","M7 21H5a2 2 0 0 1-2-2v-2","M7 12h10","M12 7v10"],
      check: ["M20 6 9 17l-5-5"],
      doublecheck: ["M18 7 9.87 15 6 11","M22 1.97 18 6 22 10"],
      clock: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z","M12 6v6l4 2"],
      download: ["M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4","M7 10l5 5 5-5","M12 15V3"],
      upload: ["M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4","M17 8l-5-5-5 5","M12 3v12"],
      close: ["M18 6 6 18","M6 6l12 12"],
      dots: ["M5 12h.01M12 12h.01M19 12h.01"],
      trash: ["M3 6h18","M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6","M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"],
      shield: ["M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"],
      lock: ["M19 11H5a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7a2 2 0 0 0-2-2z","M7 11V7a5 5 0 0 1 10 0v4"],
      globe: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z","M2 12h20","M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"],
      share: ["M4 12v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8","M16 6l-4-4-4 4","M12 2v13"],
      exit: ["M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4","M16 17l5-5-5-5","M21 12H9"],
      file: ["M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z","M13 2v7h7"],
      image: ["M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z","M15 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2z","M3 17l5-5 4 4 3-3 6 6"],
      send_file: ["M10 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4","M12 14V4","M8 8l4-4 4 4"],
      play: ["M6 4l14 8-14 8z"],
      pause: ["M6 4h4v16H6zM14 4h4v16h-4z"],
      record: ["M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z","M12 12v9"],
      videooff: ["M16 16v1a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2m5.66 0H14a2 2 0 0 1 2 2v3.34l1 1L23 7v10","M1 1l22 22"],
      groups: ["M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2","M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z","M23 21v-2a4 4 0 0 0-3-3.87","M16 3.13a4 4 0 0 1 0 7.75"],
      emoji: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z","M8 14a4 4 0 0 0 8 0","M9 9h.01M15 9h.01"],
      bolt: ["M13 2 3 14h9l-1 8 10-12h-9l1-8z"],
      key: ["M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0 3 3L22 7l-3-3m-3.5 3.5L19 4"],
    };
    const p = paths[name] ?? [];
    return `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${p.map(d=>`<path d="${escSVG(d)}"/>`).join('')}</svg>`;
  },
  avatar(name: string, size = 44, extra: string = "", online = false): string {
    const color = avatarColor(name);
    return `<div class="avatar${online?" online":""}" style="width:${size}px;height:${size}px;background:${color};font-size:${size*0.36}px" ${extra}>${esc(initials(name)||"?")}</div>`;
  },
};

/* ══════════════════════ STATE ══════════════════════ */

interface AppState {
  identity: Identity | null;
  contacts: Contact[];
  groups: Group[];
  messages: Map<string, Message[]>;
  groupMessages: Map<string, GroupMessage[]>;
  reactions: Map<string, Reaction[]>;
  typing: Map<string, TypingStatus>;
  callHistory: CallLogEntry[];
  transfers: FileTransfer[];
  settings: AppSettings | null;
  activeFilter: string;
  searchQuery: string;
  currentContact: Contact | null;
  currentGroup: Group | null;
  activeCall: CallSessionView | null;
  view: "chats" | "calls" | "settings";
}

interface CallSessionView {
  id: string;
  peer: string;
  isVideo: boolean;
  incoming: boolean;
  started: number;
  timerInterval?: ReturnType<typeof setInterval>;
}

class VchatApp {
  state: AppState = {
    identity: null, contacts: [], groups: [], messages: new Map(), groupMessages: new Map(),
    reactions: new Map(), typing: new Map(), callHistory: [], transfers: [], settings: null,
    activeFilter: "all", searchQuery: "", currentContact: null, currentGroup: null, activeCall: null,
    view: "chats",
  };

  private mediaRecorder: MediaRecorder | null = null;
  private audioChunks: Blob[] = [];
  private typingTimeout: ReturnType<typeof setTimeout> | null = null;
  private replyTarget: Message | null = null;
  private modalStack: HTMLElement[] = [];
  private localVideoStream: MediaStream | null = null;

  private qrStream: MediaStream | null = null;
  private qrAnim: number | null = null;

  constructor() {
    this.init();
  }

  /* ── Bootstrap ── */

  private async init(): Promise<void> {
    try {
      await this.initDatabase();
    } catch (e) {
      console.warn("DB init check failed, retrying…", e);
      await new Promise(r => setTimeout(r, 600));
      try { await this.initDatabase(); } catch (e2) { console.error("DB init failed:", e2); }
    }
    this.setupEventListeners();
    this.applyTheme();
    await this.loadData();
    this.render();
    this.setupSocketListener();
  }

  private async initDatabase(): Promise<void> {
    try { await api.initDb(); } catch (e) { console.warn("initDb warn:", e); }
  }

  private async loadData(): Promise<void> {
    const tasks: Promise<any>[] = [
      api.getIdentity().then(i => this.state.identity = i),
      api.getContacts().then(c => this.state.contacts = c),
      api.getGroups().then(g => this.state.groups = g),
      api.getSettings().then(s => this.state.settings = s).catch(() => {}),
    ];
    try { await Promise.allSettled(tasks); } catch (e) { console.error(e); }
    for (const c of this.state.contacts) {
      api.getMessages(c.onion_address).then(ms => this.state.messages.set(c.onion_address, ms)).catch(() => {});
    }
  }

  /* ── Event wiring ── */

  private setupEventListeners(): void {
    const $btn = (id: string, cb: () => void) => $(id)?.addEventListener("click", cb);
    $btn("#btn-settings", () => this.showModal("settings"));
    $btn("#btn-profile", () => this.showModal("profile"));
    $btn("#btn-back", () => this.closeChat());
    $btn("#btn-call", () => this.startCall(false));
    $btn("#btn-video", () => this.startCall(true));
    $btn("#btn-send", () => this.sendMessage());
    $btn("#fab-new", () => this.showModal("new-chat"));
    $btn("#fab-scan", () => this.showModal("qr-scan"));
    $btn("#btn-attach", () => this.showModal("attach"));
    $btn("#btn-voice-note", () => this.toggleVoiceNote());
    $btn("#btn-chat-menu", () => this.showModal("chat-menu"));
    $btn("#btn-start", () => this.showModal("new-chat"));

    $("#composer")?.addEventListener("input", (e) => {
      const ta = e.target as HTMLTextAreaElement;
      ta.style.height = "auto";
      ta.style.height = Math.min(ta.scrollHeight, 120) + "px";
      const $send = $("#btn-send"); if ($send) ($send as HTMLButtonElement).disabled = !ta.value.trim();
      this.notifyTyping();
    });
    $("#composer")?.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); this.sendMessage(); }
    });
    $("#search-input")?.addEventListener("input", (e) => {
      this.state.searchQuery = (e.target as HTMLInputElement).value.trim();
      this.renderChatList();
    });
    $$(".chip").forEach(ch => ch.addEventListener("click", () => {
      $$(".chip").forEach(c => c.classList.remove("active"));
      ch.classList.add("active");
      this.state.activeFilter = ch.dataset.filter || "all";
      this.renderChatList();
    }));
    $$(".nav-item").forEach(n => n.addEventListener("click", () => this.setView(n.dataset.nav as any)));
  }

  private setupSocketListener(): void {
    listen("new-message", (e: any) => {
      const msg = e.payload as Message;
      const list = this.state.messages.get(msg.sender) || [];
      list.push(msg); this.state.messages.set(msg.sender, list);
      this.renderMessages();
      this.renderChatList();
      this.toast("New message from " + msg.sender.slice(0,6));
    });
    listen("incoming-call", (e: any) => {
      this.showIncomingCall(e.payload);
    });
    listen("typing", (e: any) => {
      const t = e.payload as TypingStatus;
      this.state.typing.set(t.peer_onion, t);
      this.updateChatSubtitle();
    });
    listen("message-delivered", (e: any) => { this.renderMessages(); });
    listen("call-ended", (e: any) => { this.onCallEnded(e.payload); });
    listen("file-transfer-update", (e: any) => { this.onTransferUpdate(e.payload); });
    listen("tor-status", (e: any) => this.setConnStatus(e.payload));
  }

  /* ── Theme / UI prefs ── */

  private applyTheme(): void {
    const s = this.state.settings;
    document.documentElement.dataset.theme = s?.theme || "dark";
    document.documentElement.dataset.density = s?.density || "default";
  }

  private setConnStatus(status: string): void {
    const el = $("#conn-status");
    if (!el) return;
    const label = status === "connected" ? "P2P online" : status === "connecting" ? "Connecting…" : "Offline";
    el.textContent = label;
    el.classList.toggle("online", status === "connected");
    el.classList.toggle("offline", status === "offline");
  }

  /* ── Navigation ── */

  private setView(view: "chats" | "calls" | "settings"): void {
    this.state.view = view;
    $$(".nav-item").forEach(n => n.classList.toggle("active", n.dataset.nav === view));
    $$("#screen-chat, #screen-welcome").forEach(el => el.classList.add("hidden"));
    $(".list")!.style.display = view === "chats" ? "" : "none";
    this.renderChatList();
  }

  private openChat(contact: Contact): void {
    this.state.currentContact = contact;
    this.state.currentGroup = null;
    $("#screen-welcome")!.classList.add("hidden");
    const chat = $("#screen-chat")!; chat.classList.remove("hidden");
    $("#btn-back")!.style.display = window.innerWidth <= 900 ? "" : "none";
    $("#chat-title")!.textContent = contact.display_name;
    const av = $("#chat-avatar")!; av.innerHTML = S.avatar(contact.display_name, 44, "", false);
    this.renderMessages();
  }

  private closeChat(): void {
    this.state.currentContact = null;
    this.state.currentGroup = null;
    $("#screen-chat")!.classList.add("hidden");
    $("#screen-welcome")!.classList.remove("hidden");
    this.renderChatList();
  }

  /* ── RENDER ── */

  private render(): void {
    this.renderChatList();
    this.renderMessages();
  }

  private renderChatList(): void {
    const list = $("#chat-list");
    if (!list) return;
    const q = this.state.searchQuery.toLowerCase();
    const filter = this.state.activeFilter;
    const items: { key: string; label: string; sub: string; time: string; unread: number; online: boolean; type: string; onClick: () => void }[] = [];

    for (const c of this.state.contacts) {
      if (c.blocked) continue;
      if (q && !c.display_name.toLowerCase().includes(q) && !c.onion_address.includes(q)) continue;
      if (filter === "p2p" || filter === "all" || filter === "unread") {
        const msgs = this.state.messages.get(c.onion_address) || [];
        const last = msgs[msgs.length - 1];
        const unread = msgs.filter(m => !m.read && m.sender === c.onion_address).length;
        if (filter === "unread" && unread === 0) continue;
        items.push({
          key: c.id, label: c.display_name, sub: last ? last.content : "Tap to chat", time: last ? timeFmt(last.timestamp) : "",
          unread, online: false, type: "dm", onClick: () => this.openChat(c),
        });
      }
    }

    for (const g of this.state.groups) {
      if (q && !g.name.toLowerCase().includes(q)) continue;
      if (filter === "groups" || filter === "all" || filter === "unread") {
        items.push({ key: g.id, label: g.name, sub: `${g.member_count} members`, time: "", unread: 0, online: false, type: "group", onClick: () => this.openGroup(g) });
      }
    }

    items.sort((a,b) => (b.unread - a.unread) || b.time.localeCompare(a.time));

    if (items.length === 0) {
      list.innerHTML = `<div class="empty"><div class="empty-card"><h3>${q ? "No results" : "No conversations"}</h3><p>${q ? "Try a different search." : "Add a contact to start chatting."}</p></div></div>`;
      return;
    }

    list.innerHTML = items.map(it => `
      <div class="chat-row${it.unread ? " unread" : ""}${this.state.currentContact?.id === it.key || this.state.currentGroup?.id === it.key ? " active" : ""}" data-key="${esc(it.key)}">
        ${S.avatar(it.label, 46, "", it.online)}
        <div class="chat-meta">
          <div class="chat-topline">
            <span class="chat-title">${esc(it.label)}</span>
            <span class="chat-time">${it.time}</span>
          </div>
          <div class="chat-preview">
            <span class="chat-type ${it.type === "group" ? "groups" : ""}">${it.type === "group" ? S.icon("groups", 13) : S.icon("lock", 12)}</span>
            <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${esc(it.sub.split("\n")[0] || "⋯")}</span>
            ${it.unread ? `<span class="badge">${it.unread}</span>` : ""}
          </div>
        </div>
      </div>`).join("");

    $$(".chat-row").forEach(row => row.addEventListener("click", () => {
      const key = row.dataset.key!;
      const item = items.find(i => i.key === key);
      item?.onClick();
    }));
  }

  private renderMessages(): void {
    const box = $("#messages");
    if (!box || !(this.state.currentContact || this.state.currentGroup)) return;
    box.innerHTML = "";

    let prevDay = "";
    const contact = this.state.currentContact;
    const messages = contact ? (this.state.messages.get(contact.onion_address) || []) : [];

    for (const m of messages) {
      const day = dayFmt(m.timestamp);
      if (day !== prevDay) {
        box.insertAdjacentHTML("beforeend", `<div class="day-sep">${esc(day)}</div>`);
        prevDay = day;
      }
      const sent = m.sender === this.state.identity?.onion_address || m.sender === "local" || (m.sender === this.state.identity?.public_key);
      const ticks = m.read ? S.icon("doublecheck", 15) : m.delivered ? S.icon("check", 15) : S.icon("clock", 14);

      let body = esc(m.content);
      if (m.message_type === "file" || m.message_type === "image" || m.message_type === "video" || m.message_type === "audio" || m.message_type === "voice-note") {
        body = this.renderAttachment(m);
      }

      box.insertAdjacentHTML("beforeend", `
        <div class="bubble-row ${sent ? "sent" : "recv"}" data-id="${esc(m.id)}">
          <div class="bubble">
            ${body}
            <div class="bubble-meta">
              ${timeFmt(m.timestamp)}
              ${sent ? `<span class="ticks">${ticks}</span>` : ""}
            </div>
          </div>
        </div>`);
    }
    box.scrollTop = box.scrollHeight;
  }

  private renderAttachment(m: Message): string {
    const isImg = m.message_type === "image";
    const isFile = m.message_type === "file";
    const isVoice = m.message_type === "voice-note";
    if (isImg) return `<img src="${esc(m.content)}" style="max-width:260px;max-height:220px;border-radius:10px;display:block"/>`;
    if (isVoice) return `
      <div style="display:flex;align-items:center;gap:8px;min-width:180px">
        <button class="icon-btn" style="flex-shrink:0">${S.icon("play", 18)}</button>
        <div style="flex:1;height:4px;background:rgba(128,128,128,.3);border-radius:2px"><div style="width:35%;height:100%;background:currentColor;border-radius:2px"></div></div>
        <span style="font-size:11px;opacity:.8">0:07</span>
      </div>`;
    if (isFile) return `
      <div style="display:flex;align-items:center;gap:10px;min-width:180px">
        ${S.icon("file", 22)}
        <div style="flex:1;min-width:0">
          <div style="font-weight:600;font-size:13px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis">${esc(m.content)}</div>
          <div style="font-size:11px;opacity:.7">${fmtBytes(0)}</div>
        </div>
        ${S.icon("download", 18)}
      </div>`;
    return `<span>${esc(m.content)}</span>`;
  }

  private updateChatSubtitle(): void {
    const el = $("#chat-subtitle");
    if (!el) return;
    const c = this.state.currentContact;
    if (!c) { el.textContent = "Select a conversation"; return; }
    const t = this.state.typing.get(c.onion_address);
    if (t?.is_typing) { el.textContent = "typing…"; el.style.color = "var(--accent)"; return; }
    el.textContent = c.verified ? "✓ E2E encrypted · PQC" : "End-to-end encrypted";
    el.style.color = "";
  }

  /* ── Messages ── */

  private async sendMessage(): Promise<void> {
    const ta = $("#composer") as HTMLTextAreaElement;
    if (!ta) return;
    const content = ta.value.trim();
    if (!content) return;
    ta.value = ""; ta.style.height = "auto";
    $("#btn-send")!.setAttribute("disabled", "");

    const contact = this.state.currentContact;
    if (!contact) { this.toast("Select a conversation first"); return; }

    try {
      const msg = await api.sendMessage(contact.onion_address, content, this.replyTarget?.id ?? "");
      const list = this.state.messages.get(contact.onion_address) || [];
      list.push(msg); this.state.messages.set(contact.onion_address, list);
      this.replyTarget = null;
      this.renderMessages(); this.renderChatList();
    } catch (e) {
      this.toast("Failed to send: " + e);
    }
  }

  private notifyTyping(): void {
    const c = this.state.currentContact;
    if (!c) return;
    if (this.typingTimeout) clearTimeout(this.typingTimeout);
    api.sendTypingIndicator(c.onion_address, true).catch(() => {});
    this.typingTimeout = setTimeout(() => {
      api.sendTypingIndicator(c.onion_address, false).catch(() => {});
    }, 2000);
  }

  /* ── Voice notes ── */

  private async toggleVoiceNote(): Promise<void> {
    if (this.mediaRecorder && this.mediaRecorder.state === "recording") {
      this.mediaRecorder.stop();
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      this.mediaRecorder = new MediaRecorder(stream);
      this.audioChunks = [];
      this.recordingStart = Date.now();
      this.mediaRecorder.ondataavailable = (e) => this.audioChunks.push(e.data);
      this.mediaRecorder.onstop = async () => {
        const blob = new Blob(this.audioChunks, { type: "audio/webm" });
        const c = this.state.currentContact;
        if (c) { await api.sendVoiceNote(c.onion_address, blob).catch(err => this.toast(String(err))); }
        stream.getTracks().forEach(t => t.stop());
      };
      this.mediaRecorder.start();
      this.toast("Recording… tap stop when done");
      const btn = $("#btn-voice-note");
      btn?.classList.add("active");
    } catch (e) {
      this.toast("Microphone unavailable: " + e);
    }
  }

  /* ── Calls ── */

  private async startCall(isVideo: boolean): Promise<void> {
    const c = this.state.currentContact;
    if (!c) { this.toast("Select a contact first"); return; }
    try {
      const id = isVideo ? await api.startVideoCall(c.onion_address) : await api.startAudioCall(c.onion_address);
      this.state.activeCall = { id, peer: c.onion_address, isVideo, incoming: false, started: Date.now() };
      this.showCallOverlay();
    } catch (e) {
      this.toast("Call failed: " + e);
    }
  }

  private showIncomingCall(payload: any): void {
    const { call_id, peer_onion, is_video, from_name } = payload ?? {};
    this.state.activeCall = { id: call_id, peer: peer_onion, isVideo: !!is_video, incoming: true, started: Date.now() };
    const root = $("#call-overlay-root")!;
    root.innerHTML = `
      <div class="call-overlay">
        <div class="call-video" style="place-items:center">
          <div style="text-align:center;padding:24px">
            ${S.avatar(from_name || shortOnion(peer_onion) || "?", 96)}
            <h2 style="margin-top:16px">${esc(from_name || shortOnion(peer_onion))}</h2>
            <p style="opacity:.7">${is_video ? "Incoming video call" : "Incoming audio call"}</p>
            <p style="opacity:.5;font-size:12px;margin-top:8px">${S.icon(is_video ? "lock" : "shield", 14)} End-to-end encrypted</p>
          </div>
        </div>
        <div class="call-controls">
          <button class="call-btn end" id="call-reject" title="Decline">${S.icon("close", 24)}</button>
          <button class="call-btn" id="call-accept" title="Accept" style="background:var(--success);border-color:var(--success)">${S.icon(is_video ? "camera" : "call", 24)}</button>
        </div>
      </div>`;
    $("#call-reject")!.addEventListener("click", () => { api.rejectCall(this.state.activeCall!.id).catch(()=>{}); this.dismissCallOverlay(); });
    $("#call-accept")!.addEventListener("click", () => {
      this.state.activeCall!.incoming = false;
      api.answerVideoCall(this.state.activeCall!.id).catch(()=>{});
      this.showCallOverlay();
    });
  }

  private showCallOverlay(): void {
    const call = this.state.activeCall;
    if (!call) return;
    const root = $("#call-overlay-root")!;
    const name = this.state.contacts.find(c => c.onion_address === call.peer)?.display_name || shortOnion(call.peer);
    root.innerHTML = `
      <div class="call-overlay">
        <div class="call-video">
          <video id="call-remote-video" autoplay playsinline muted></video>
          ${call.isVideo ? `<video id="call-local-video" autoplay playsinline muted style="position:absolute;bottom:16px;right:16px;width:110px;height:150px;border-radius:14px;object-fit:cover;box-shadow:0 8px 24px rgba(0,0,0,.5)"></video>` : ""}
          <div style="position:absolute;bottom:16px;left:16px;text-align:left">
            <h2>${esc(name)}</h2>
            <p style="opacity:.7;display:flex;align-items:center;gap:6px"><span class="call-timer">0:00</span> · ${S.icon("lock", 14)} ${call.isVideo ? "Video" : "Audio"} encrypted</p>
          </div>
        </div>
        <div class="call-controls">
          <button class="call-btn active" id="call-mute" title="Mute">${S.icon("mic", 24)}</button>
          ${call.isVideo ? `<button class="call-btn active" id="call-cam" title="Camera">${S.icon("video", 24)}</button>` : ""}
          <button class="call-btn end" id="call-end" title="Hang up">${S.icon("close", 26)}</button>
        </div>
      </div>`;
    call.timerInterval = setInterval(() => {
      const secs = Math.floor((Date.now() - call.started) / 1000);
      const t = $(`.call-timer`); if (t) t.textContent = `${Math.floor(secs/60)}:${String(secs%60).padStart(2,'0')}`;
    }, 1000);
    $("#call-end")!.addEventListener("click", () => { api.endVideoCall(call.id).catch(()=>{}); this.onCallEnded(call.id); });
    $("#call-mute")!.addEventListener("click", (e) => {
      const btn = e.currentTarget as HTMLElement;
      btn.classList.toggle("active");
      btn.innerHTML = btn.classList.contains("active") ? S.icon("mic", 24) : S.icon("mic", 24);
    });
    $("#call-cam")?.addEventListener("click", (e) => {
      const btn = e.currentTarget as HTMLElement;
      btn.classList.toggle("active");
    });
    if (call.isVideo) this.startLocalVideo();
  }

  private async startLocalVideo(): Promise<void> {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ video: { width: 640, height: 480 }, audio: false });
      this.localVideoStream = stream;
      const v = $("#call-local-video") as HTMLVideoElement;
      if (v) { v.srcObject = stream; v.play().catch(()=>{}); }
    } catch (e) { console.warn("Camera unavailable:", e); }
  }

  private onCallEnded(id: string): void {
    const call = this.state.activeCall;
    if (call?.id === id || (id && !this.state.activeCall?.id)) {
      this.dismissCallOverlay();
    }
  }

  private dismissCallOverlay(): void {
    const call = this.state.activeCall;
    if (call?.timerInterval) clearInterval(call.timerInterval);
    if (this.localVideoStream) { this.localVideoStream.getTracks().forEach(t => t.stop()); this.localVideoStream = null; }
    this.state.activeCall = null;
    $("#call-overlay-root")!.innerHTML = "";
  }

  /* ── Modals ── */

  private showModal(kind: string, _data?: any): void {
    const root = $("#modal-root")!;
    let html = "";
    let id = "";

    switch (kind) {
      case "settings": {
        id = "modal-settings"; const s = this.state.settings || { theme: "dark", density: "default" } as any;
        html = this.settingsModal(s);
        break;
      }
      case "profile": {
        id = "modal-profile";
        const me = this.state.identity;
        html = `
          <div class="modal-head"><h3>My profile</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
          <div class="modal-body">
            <div style="display:flex;flex-direction:column;align-items:center;gap:12px;padding:8px">
              ${S.avatar(me?.display_name || "?", 84)}
              <h2 style="font-size:20px">${esc(me?.display_name || "Unknown")}</h2>
              <div class="pill"><span class="dot" style="background:var(--success)"></span> P2P online</div>
            </div>
            <div class="card" style="padding:14px;display:flex;flex-direction:column;gap:10px">
              <div><div class="label">Your vchat ID</div>
                <div class="pill" style="cursor:pointer" id="copy-onion">${S.icon("key", 14)} ${esc(shortOnion(me?.onion_address || ""))}</div>
              </div>
              <div class="label">Share via QR</div>
              <button class="btn primary" id="show-qr">${S.icon("qr", 18)} Show QR code</button>
            </div>
            <button class="btn" id="btn-backup">Export backup (later)</button>
          </div>`;
        break;
      }
      case "new-chat": {
        id = "modal-new-chat";
        html = `
          <div class="modal-head"><h3>New chat</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
          <div class="modal-body">
            <button class="btn primary" id="new-add-contact" style="width:100%;justify-content:flex-start">${S.icon("user", 18)} Add contact</button>
            <button class="btn" id="new-create-group" style="width:100%;justify-content:flex-start">${S.icon("groups", 18)} Create group</button>
            <button class="btn" id="new-scan-qr" style="width:100%;justify-content:flex-start">${S.icon("qr", 18)} Scan QR code</button>
          </div>`;
        break;
      }
      case "qr-scan": {
        id = "modal-qr-scan";
        html = `
          <div class="modal-head"><h3>Scan QR code</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
          <div class="modal-body">
            <video id="qr-video" style="width:100%;border-radius:14px;background:#000" playsinline muted></video>
            <canvas id="qr-canvas" hidden></canvas>
            <p style="font-size:12px;color:var(--text-2);text-align:center">Point your camera at a vchat QR code</p>
            <div style="display:flex;gap:8px">
              <button class="btn ghost" id="qr-paste">${S.icon("upload", 16)} Paste image</button>
              <button class="btn" id="qr-cancel">Cancel</button>
            </div>
          </div>`;
        break;
      }
      case "attach": {
        id = "modal-attach";
        html = `
          <div class="modal-head"><h3>Attach</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
          <div class="modal-body">
            <button class="btn" id="att-file" style="width:100%;justify-content:flex-start">${S.icon("file", 18)} Send file</button>
            <button class="btn" id="att-image" style="width:100%;justify-content:flex-start">${S.icon("image", 18)} Send image</button>
            <button class="btn" id="att-voice" style="width:100%;justify-content:flex-start">${S.icon("mic", 18)} Record voice note</button>
          </div>`;
        break;
      }
      case "chat-menu": {
        id = "modal-chat-menu";
        const c = this.state.currentContact;
        html = `
          <div class="modal-head"><h3>${esc(c?.display_name || "")}</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
          <div class="modal-body">
            <div class="card" style="padding:14px;display:flex;flex-direction:column;gap:12px">
              <div style="display:flex;align-items:center;gap:10px">${S.icon("shield", 20)} <div><div class="label" style="margin:0">Encryption</div><div style="font-size:13px">${c?.verified ? "Verified · post-quantum" : "E2E encrypted · PQC"}</div></div></div>
              <div style="display:flex;align-items:center;gap:10px">${S.icon("globe", 20)} <div><div class="label" style="margin:0">vchat ID</div><div style="font-size:13px;font-family:var(--mono)">${esc(c?.onion_address || "")}</div></div></div>
              <button class="btn" id="menu-clear">${S.icon("trash", 18)} Clear messages</button>
              <button class="btn" id="menu-block" style="color:var(--danger)">Block contact</button>
            </div>
          </div>`;
        break;
      }
      default: return;
    }

    const modal = document.createElement("div");
    modal.className = "modal-backdrop";
    modal.id = "backdrop-" + id;
    modal.innerHTML = `<div class="modal" id="${id}">${html}</div>`;
    root.appendChild(modal);
    this.modalStack.push(modal);
    this.wireModal(modal, kind);

    // post-render wiring
    switch (kind) {
      case "settings": this.wireSettings(modal); break;
      case "profile": {
        const qrBtn = modal.querySelector("#show-qr");
        qrBtn?.addEventListener("click", async () => {
          try { const svg = await api.generateQRCode(); this.showQr(svg); } catch (e) { this.toast(String(e)); }
        });
        const copyBtn = modal.querySelector("#copy-onion");
        copyBtn?.addEventListener("click", () => { navigator.clipboard?.writeText(this.state.identity?.onion_address || "").then(() => this.toast("ID copied")).catch(()=>{}); });
        break;
      }
      case "new-chat": {
        modal.querySelector("#new-add-contact")?.addEventListener("click", () => this.showModal("add-contact"));
        modal.querySelector("#new-create-group")?.addEventListener("click", () => this.showModal("create-group"));
        modal.querySelector("#new-scan-qr")?.addEventListener("click", () => this.showModal("qr-scan"));
        break;
      }
      case "qr-scan": this.startQrScan(modal); break;
      case "attach": {
        modal.querySelector("#att-file")?.addEventListener("click", async () => {
          const file = await this.pickFile();
          if (file) this.sendFile(file);
        });
        modal.querySelector("#att-image")?.addEventListener("click", async () => {
          const file = await this.pickFile(true);
          if (file) this.sendFile(file, true);
        });
        modal.querySelector("#att-voice")?.addEventListener("click", () => { this.toggleVoiceNote(); });
        break;
      }
      case "chat-menu": {
        modal.querySelector("#menu-clear")?.addEventListener("click", () => this.toast("Clearing…"));
        modal.querySelector("#menu-block")?.addEventListener("click", () => {
          const c = this.state.currentContact;
          if (c) { api.blockContact(c.id).then(()=>{ this.toast("Contact blocked"); this.state.contacts = this.state.contacts.filter(x=>x.id!==c.id); this.renderChatList(); }).catch(e=>this.toast(String(e))); }
        });
        break;
      }
    }
  }

  private wireModal(modal: HTMLElement, _kind: string): void {
    modal.querySelector(".modal-close")?.addEventListener("click", () => this.closeModal());
    modal.addEventListener("click", (e) => { if (e.target === modal) this.closeModal(); });
  }

  private closeModal(): void {
    const modal = this.modalStack.pop();
    if (!modal) return;
    if (modal.id === "backdrop-modal-qr-scan") this.stopQrScan();
    modal.style.animation = "fadeOut .2s ease-out";
    setTimeout(() => modal.remove(), 200);
  }

  private settingsModal(s: any): string {
    const themes = ["dark","light","amoled","ocean"];
    const densities = ["default","compact","comfortable"];
    return `
      <div class="modal-head"><h3>Settings</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
      <div class="modal-body">
        <div class="label">Theme</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          ${themes.map(t => `<button class="chip theme-opt ${s?.theme === t ? "active" : ""}" data-theme-val="${t}">${esc(t)}</button>`).join('')}
        </div>
        <div class="label" style="margin-top:12px">Density</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          ${densities.map(d => `<button class="chip density-opt ${s?.density === d ? "active" : ""}" data-density-val="${d}">${esc(d)}</button>`).join('')}
        </div>
        <div style="display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:12px">
          <div><div class="label" style="margin:0">Notifications</div><div style="font-size:12px;color:var(--text-2)">Incoming alerts</div></div>
          <input type="checkbox" class="toggle-chk" data-setting="notifications" ${s?.notifications === false ? "" : "checked"} style="width:44px;height:24px;accent-color:var(--accent)">
        </div>
        <div class="card" style="padding:14px;display:flex;align-items:center;gap:12px;margin-top:12px">
          ${S.icon("shield", 22)}
          <div style="flex:1"><div class="label" style="margin:0">Encryption</div><div style="font-size:12px;color:var(--text-2)">Post-quantum · ML-KEM-768 + X25519 · AES-256-GCM</div></div>
          <span class="pill online"><span class="dot"></span> Active</span>
        </div>
      </div>`;
  }

  private wireSettings(modal: HTMLElement): void {
    modal.querySelectorAll(".theme-opt").forEach((b: HTMLElement) => b.addEventListener("click", async () => {
      const t = b.dataset.themeVal!;
      modal.querySelectorAll(".theme-opt").forEach(x => x.classList.toggle("active", x === b));
      document.documentElement.dataset.theme = t;
      const s = this.state.settings as any || {};
      s.theme = t; this.state.settings = s;
      await api.updateSettings(s).catch(()=>{});
    }));
    modal.querySelectorAll(".density-opt").forEach((b: HTMLElement) => b.addEventListener("click", async () => {
      const d = b.dataset.densityVal!;
      modal.querySelectorAll(".density-opt").forEach(x => x.classList.toggle("active", x === b));
      document.documentElement.dataset.density = d;
      const s = this.state.settings as any || {};
      s.density = d; this.state.settings = s;
      await api.updateSettings(s).catch(()=>{});
    }));
    modal.querySelector(".toggle-chk")?.addEventListener("change", async (e) => {
      const s = this.state.settings as any || {};
      s.notifications = (e.target as HTMLInputElement).checked;
      await api.updateSettings(s).catch(()=>{});
    });
  }

  private showQr(svgData: string): void {
    const root = $("#modal-root")!;
    const modal = document.createElement("div");
    modal.className = "modal-backdrop";
    modal.innerHTML = `
      <div class="modal" style="max-width:340px">
        <div class="modal-head"><h3>My QR code</h3><button class="icon-btn modal-close">${S.icon("close", 18)}</button></div>
        <div class="modal-body" style="align-items:center">
          <div style="width:100%;" id="qr-svg">${svgData}</div>
          <p style="font-size:12px;color:var(--text-2);text-align:center">Scan this to add your contact</p>
        </div>
      </div>`;
    root.appendChild(modal);
    modal.querySelector(".modal-close")?.addEventListener("click", () => modal.remove());
    modal.addEventListener("click", e => { if (e.target === modal) modal.remove(); });
  }

  /* ── QR scanning ── */

  private async startQrScan(modal: HTMLElement): Promise<void> {
    try {
      this.qrStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
      if (!modal.isConnected) { this.qrStream.getTracks().forEach(t=>t.stop()); return; }
      const video = modal.querySelector("#qr-video") as HTMLVideoElement;
      const canvas = modal.querySelector("#qr-canvas") as HTMLCanvasElement;
      video.srcObject = this.qrStream;
      await video.play();
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      const tick = () => {
        if (!modal.isConnected) return;
        if (video.readyState >= 2 && ctx) {
          canvas.width = video.videoWidth; canvas.height = video.videoHeight;
          ctx.drawImage(video, 0, 0);
          const img = ctx.getImageData(0, 0, canvas.width, canvas.height);
          const code = jsQR(img.data, img.width, img.height, { inversionAttempts: "dontInvert" });
          if (code?.data) {
            this.handleScannedQr(code.data, modal);
            return;
          }
        }
        this.qrAnim = requestAnimationFrame(tick);
      };
      this.qrAnim = requestAnimationFrame(tick);
    } catch (e) {
      this.toast("Camera unavailable — paste an image instead");
      const pasteBtn = modal.querySelector("#qr-paste");
      pasteBtn?.addEventListener("click", () => {
        const input = document.createElement("input");
        input.type = "file"; input.accept = "image/*";
        input.onchange = async () => {
          const f = input.files?.[0]; if (!f) return;
          const img = await createImageBitmap(f);
          const canvas = document.createElement("canvas");
          canvas.width = img.width; canvas.height = img.height;
          const ctx = canvas.getContext("2d")!;
          ctx.drawImage(img, 0, 0);
          const code = jsQR(ctx.getImageData(0,0,canvas.width,canvas.height).data, canvas.width, canvas.height, { inversionAttempts: "dontInvert" });
          if (code?.data) this.handleScannedQr(code.data, modal);
          else this.toast("No QR code found in image");
        };
        input.click();
      });
    }
  }

  private stopQrScan(): void {
    if (this.qrAnim) cancelAnimationFrame(this.qrAnim);
    if (this.qrStream) { this.qrStream.getTracks().forEach(t => t.stop()); this.qrStream = null; }
  }

  private handleScannedQr(data: string, modal: HTMLElement): void {
    this.stopQrScan();
    this.toast("QR scanned!");
    try {
      this.processQrData(data, modal);
    } catch (e) {
      this.toast("Invalid QR: " + e);
    }
  }

  private async processQrData(data: string, modal: HTMLElement): Promise<void> {
    try {
      const contact = await api.parseAndAddQr(data);
      this.state.contacts.push(contact);
      this.renderChatList();
      this.closeModal();
      this.toast("Contact added: " + contact.display_name);
    } catch (e) {
      // fallback: open add-contact prefilled
      this.toast("Parsing…");
      this.closeModal();
      this.showModal("add-contact", { prefilled: data });
    }
  }

  /* ── Files ── */

  private pickFile(imageOnly = false): Promise<File | null> {
    return new Promise((resolve) => {
      const input = document.createElement("input");
      input.type = "file";
      if (imageOnly) input.accept = "image/*";
      input.onchange = () => resolve(input.files?.[0] || null);
      input.click();
    });
  }

  private async sendFile(file: File, _asImage = false): Promise<void> {
    const c = this.state.currentContact;
    if (!c) return;
    try {
      const data = new Uint8Array(await file.arrayBuffer());
      await api.sendFile(c.onion_address, file.name, data);
      this.toast("File sent");
    } catch (e) { this.toast("File send failed: " + e); }
  }

  /* ── Transfers ── */

  private onTransferUpdate(payload: any): void {
    this.toast(`Transfer ${payload?.status || "updated"}`);
  }

  /* ── Group ── */

  private openGroup(g: Group): void {
    this.state.currentGroup = g;
    this.state.currentContact = null;
    $("#screen-welcome")!.classList.add("hidden");
    const chat = $("#screen-chat")!; chat.classList.remove("hidden");
    $("#btn-back")!.style.display = window.innerWidth <= 900 ? "" : "none";
    $("#chat-title")!.textContent = g.name;
    $("#chat-avatar")!.innerHTML = S.avatar(g.name, 44, "", false);
    $("#chat-subtitle")!.textContent = `Group · ${g.member_count} members`;
    this.renderGroupMessages(g);
  }

  private renderGroupMessages(g: Group): void {
    const box = $("#messages");
    if (!box) return;
    box.innerHTML = "";
    this.state.groupMessages.get(g.id)?.forEach(m => {
      box.insertAdjacentHTML("beforeend", `<div class="bubble-row recv"><div class="bubble">${esc(m.content)}<div class="bubble-meta">${timeFmt(m.timestamp)}</div></div></div>`);
    });
    box.scrollTop = box.scrollHeight;
  }

  /* ── Toast ── */

  private toast(msg: string): void {
    const stack = $("#toast-stack");
    if (!stack) return;
    const el = document.createElement("div");
    el.className = "toast";
    el.textContent = msg;
    stack.appendChild(el);
    setTimeout(() => {
      el.style.opacity = "0";
      el.style.transition = "opacity .3s";
      setTimeout(() => el.remove(), 320);
    }, 2600);
  }
}

/* ══════════════════════ BOOTSTRAP ══════════════════════ */

const app = new VchatApp();
(window as any).vchat = app;

// Hot-reload safe
if (typeof (window as any).__vchat_setup_done === "undefined") {
  (window as any).__vchat_setup_done = true;
}

export { VchatApp };