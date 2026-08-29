import { listen } from "@tauri-apps/api/event";
import jsQR from "jsqr";
import { api, Contact, Message, Identity, Group, GroupMessage, GroupMember, Reaction, TypingStatus, CallLogEntry } from "./lib/api";
import { store } from "./lib/store";

class VchatApp {
  currentContact: Contact | null = null;
  currentGroup: Group | null = null;
  activeCallId: string | null = null;
  callTimerInterval: any = null;
  callSeconds: number = 0;
  replyToMessage: Message | null = null;
  typingTimeout: any = null;
  contextMenuTarget: Message | null = null;
  mediaRecorder: MediaRecorder | null = null;
  audioChunks: Blob[] = [];
  recordingStartTime: number = 0;
  callMediaStream: MediaStream | null = null;
  callSeq: number = 0;
  callVideoTimer: any = null;
  remoteAudioCtx: AudioContext | null = null;
  remoteVideoCanvas: HTMLCanvasElement | null = null;
  remoteVideoCtx: CanvasRenderingContext2D | null = null;
  isIncomingCall: boolean = false;
  incomingPeerOnion: string | null = null;
  activeCallPeerOnion: string | null = null;
  activeCallIsVideo: boolean = false;
  screenStream: MediaStream | null = null;
  voiceQ: Blob[] = [];
  voicePlaying: boolean = false;

  async init(): Promise<void> {
    try {
      await this.initDatabase();
    } catch (err) {
      console.warn("Database init check failed (retrying):", err);
      await new Promise((r) => setTimeout(r, 500));
      try {
        await this.initDatabase();
      } catch (err2) {
        console.error("Database init failed permanently:", err2);
      }
    }

    // ALWAYS set up UI regardless of data loading status
    this.setupNavigation();
    this.setupChatView();
    this.setupGroupChatView();
    this.setupModals();
    this.setupCallOverlay();
    this.setupContextMenu();
    this.setupEmojiPicker();
    this.setupSearch();
    this.setupEventListeners();
    this.setupSettings();
    this.showScreen("chats");

    // Load data — each is independent, failures are non-fatal
    await this.loadIdentity().catch((e) => console.warn("Identity load:", e));
    await this.loadContacts().catch((e) => console.warn("Contacts load:", e));
    await this.loadGroups().catch((e) => console.warn("Groups load:", e));
    await this.loadCallHistory().catch((e) => console.warn("Calls load:", e));
    await this.loadSettings().catch((e) => console.warn("Settings load:", e));
    await this.updateTorStatus().catch((e) => console.warn("Tor status:", e));

    setInterval(() => this.updateTorStatus(), 30000);
    setInterval(() => this.updateTypingIndicators(), 5000);
  }

  async initDatabase(): Promise<void> {
    await api.getContacts();
  }

  async loadIdentity(): Promise<void> {
    try {
      let identity: Identity | null = null;
      try {
        identity = await api.getIdentity();
      } catch {
        // command failed
      }
      if (!identity) {
        identity = await api.initIdentity("User");
      }
      if (identity) {
        store.setIdentity(identity);
        this.updateSettingsProfile(identity);
      }
    } catch (err) {
      console.error("Failed to load identity:", err);
    }
  }

  updateSettingsProfile(identity: Identity): void {
    const avatarEl = document.getElementById("settings-avatar");
    const nameEl = document.getElementById("settings-name");
    const onionEl = document.getElementById("settings-onion");
    if (avatarEl) {
      avatarEl.textContent = identity.display_name.charAt(0).toUpperCase();
      avatarEl.className = `avatar avatar-large ${this.avatarColor(identity.display_name)}`;
    }
    if (nameEl) (nameEl as HTMLInputElement).value = identity.display_name;
    if (onionEl) onionEl.textContent = identity.onion_address || "Not connected";
  }

  async updateTorStatus(): Promise<void> {
    try {
      const status = await api.getTorStatus();
      const badge = document.getElementById("tor-status-badge");
      if (badge) {
        const connected = status.connected;
        badge.className = `tor-badge tor-${connected ? "connected" : "disconnected"}`;
        badge.textContent = connected ? "Tor Connected" : "Tor Offline";
      }
    } catch (err) {
      console.error("Tor status check failed:", err);
    }
  }

  async loadContacts(): Promise<void> {
    try {
      const contacts = await api.getContacts();
      store.setContacts(contacts);
      this.renderChatList(contacts);
      this.renderContacts(contacts);
    } catch (err) {
      console.error("Failed to load contacts:", err);
    }
  }

  async loadGroups(): Promise<void> {
    try {
      const groups = await api.getGroups();
      store.setGroups(groups);
      this.renderGroupsList(groups);
    } catch (err) {
      console.error("Failed to load groups:", err);
    }
  }

  async loadCallHistory(): Promise<void> {
    try {
      const calls = await api.getCallHistory();
      store.setCallHistory(calls);
      this.renderCallHistory(calls);
    } catch (err) {
      console.error("Failed to load call history:", err);
    }
  }

  async loadSettings(): Promise<void> {
    try {
      const settings = await api.getSettings();
      const toggleMap: Record<string, string> = {
        disappearing_messages_default: "toggle-disappearing",
        read_receipts: "toggle-read-receipts",
        typing_indicators: "toggle-typing-indicators",
        notifications_enabled: "toggle-notifications",
      };
      for (const [key, elId] of Object.entries(toggleMap)) {
        const el = document.getElementById(elId) as HTMLInputElement | null;
        if (el) el.checked = (settings as any)[key];
      }
      const themeSelect = document.getElementById("settings-theme") as HTMLSelectElement | null;
      if (themeSelect) themeSelect.value = settings.theme;
    } catch (err) {
      console.error("Failed to load settings:", err);
    }
  }

  setupNavigation(): void {
    document.querySelectorAll(".nav-item").forEach((item) => {
      item.addEventListener("click", () => {
        const screen = item.getAttribute("data-screen");
        if (screen) this.showScreen(screen);
      });
    });

    document.getElementById("fab-add-contact")?.addEventListener("click", () => {
      this.showModal("add-contact-modal");
    });

    document.getElementById("fab-create-group")?.addEventListener("click", () => {
      this.showModal("create-group-modal");
    });

    document.getElementById("fab-scan-qr")?.addEventListener("click", () => {
      this.showModal("qr-code-modal");
    });

    document.getElementById("chats-search-toggle")?.addEventListener("click", () => {
      const bar = document.getElementById("chats-search-bar");
      if (bar) bar.classList.toggle("hidden");
    });

    document.getElementById("contacts-search-toggle")?.addEventListener("click", () => {
      const bar = document.getElementById("contacts-search-bar");
      if (bar) bar.classList.toggle("hidden");
    });
  }

  showScreen(name: string): void {
    document.querySelectorAll(".screen").forEach((s) => s.classList.add("hidden"));
    const target = document.getElementById(`screen-${name}`);
    if (target) target.classList.remove("hidden");

    document.querySelectorAll(".nav-item").forEach((item) => {
      item.classList.toggle("active", item.getAttribute("data-screen") === name);
    });
  }

  renderChatList(contacts: Contact[]): void {
    const container = document.getElementById("chat-list");
    if (!container) return;

    if (contacts.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">💬</div>
          <h3>No conversations yet</h3>
          <p>Add a contact to start chatting</p>
        </div>`;
      return;
    }

    container.innerHTML = contacts
      .map((c) => {
        const msgs = store.getMessagesForContact(c.onion_address);
        const lastMsg = msgs.length > 0 ? msgs[msgs.length - 1] : null;
        const preview = lastMsg ? this.esc(lastMsg.content || "Attachment") : "No messages yet";
        const time = lastMsg ? this.formatDate(lastMsg.timestamp) : "";
        const unread = msgs.filter((m) => m.sender === c.onion_address && !m.read).length;
        return `
        <div class="chat-list-item" data-onion="${this.esc(c.onion_address)}">
          <div class="avatar ${this.avatarColor(c.display_name)}">${c.display_name.charAt(0).toUpperCase()}</div>
          <div class="chat-list-info">
            <div class="chat-list-top">
              <span class="chat-list-name">${this.esc(c.display_name)}</span>
              <span class="chat-list-time">${time}</span>
            </div>
            <div class="chat-list-bottom">
              <span class="chat-list-preview">${preview}</span>
              ${unread > 0 ? `<span class="unread-badge">${unread}</span>` : ""}
            </div>
          </div>
        </div>`;
      })
      .join("");

    container.querySelectorAll(".chat-list-item").forEach((el) => {
      el.addEventListener("click", () => {
        const onion = el.getAttribute("data-onion");
        const contact = contacts.find((c) => c.onion_address === onion);
        if (contact) this.openChat(contact);
      });
    });
  }

  renderContacts(contacts: Contact[]): void {
    const container = document.getElementById("contacts-list");
    if (!container) return;

    if (contacts.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">👤</div>
          <h3>No contacts</h3>
          <p>Tap + to add a contact</p>
        </div>`;
      return;
    }

    container.innerHTML = contacts
      .map(
        (c) => `
      <div class="contact-list-item" data-onion="${this.esc(c.onion_address)}">
        <div class="avatar ${this.avatarColor(c.display_name)}">${c.display_name.charAt(0).toUpperCase()}</div>
        <div class="contact-list-info">
          <span class="contact-list-name">${this.esc(c.display_name)}</span>
          <span class="contact-list-onion">${this.esc(c.onion_address)}</span>
        </div>
      </div>`
      )
      .join("");

    container.querySelectorAll(".contact-list-item").forEach((el) => {
      el.addEventListener("click", () => {
        const onion = el.getAttribute("data-onion");
        const contact = contacts.find((c) => c.onion_address === onion);
        if (contact) this.openChat(contact);
      });
    });
  }

  renderGroupsList(groups: Group[]): void {
    const container = document.getElementById("groups-list");
    if (!container) return;

    if (groups.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">👥</div>
          <h3>No groups</h3>
          <p>Create a group to get started</p>
        </div>`;
      return;
    }

    container.innerHTML = groups
      .map(
        (g) => `
      <div class="group-list-item" data-group-id="${this.esc(g.id)}">
        <div class="avatar avatar-group">${g.name.charAt(0).toUpperCase()}</div>
        <div class="group-list-info">
          <span class="group-list-name">${this.esc(g.name)}</span>
          <span class="group-list-members">${g.member_count} member${g.member_count !== 1 ? "s" : ""}</span>
        </div>
      </div>`
      )
      .join("");

    container.querySelectorAll(".group-list-item").forEach((el) => {
      el.addEventListener("click", () => {
        const gid = el.getAttribute("data-group-id");
        const group = groups.find((g) => g.id === gid);
        if (group) this.openGroupChat(group);
      });
    });
  }

  renderCallHistory(calls: CallLogEntry[]): void {
    const container = document.getElementById("call-history-list");
    if (!container) return;

    if (calls.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">📞</div>
          <h3>No calls yet</h3>
        </div>`;
      return;
    }

    container.innerHTML = calls
      .map((call) => {
        const dirIcon = call.direction === "outgoing" ? "↗" : call.status === "missed" ? "✕" : "↙";
        const typeIcon = call.call_type === "video" ? "📹" : "📞";
        return `
        <div class="call-history-item" data-call-id="${this.esc(call.id)}">
          <div class="call-icon ${call.status === "missed" ? "missed" : ""}">${dirIcon}</div>
          <div class="call-type-icon">${typeIcon}</div>
          <div class="call-info">
            <span class="call-name">${this.esc(call.peer_onion)}</span>
            <span class="call-date">${this.formatDate(call.started_at)}</span>
          </div>
          <span class="call-duration">${call.duration_secs ? this.formatDuration(call.duration_secs) : ""}</span>
        </div>`;
      })
      .join("");

    const tabAll = document.getElementById("calls-tab-all");
    const tabMissed = document.getElementById("calls-tab-missed");
    const tabOutgoing = document.getElementById("calls-tab-outgoing");

    const filterCalls = (filter: string) => {
      const items = container.querySelectorAll(".call-history-item");
      items.forEach((item) => {
        const callId = item.getAttribute("data-call-id");
        const call = calls.find((c) => c.id === callId);
        if (!call) return;
        if (filter === "all") {
          (item as HTMLElement).style.display = "";
        } else if (filter === "missed") {
          (item as HTMLElement).style.display = call.status === "missed" ? "" : "none";
        } else if (filter === "outgoing") {
          (item as HTMLElement).style.display = call.direction === "outgoing" ? "" : "none";
        }
      });
    };

    tabAll?.addEventListener("click", () => filterCalls("all"));
    tabMissed?.addEventListener("click", () => filterCalls("missed"));
    tabOutgoing?.addEventListener("click", () => filterCalls("outgoing"));
  }

  async openChat(contact: Contact): Promise<void> {
    this.currentContact = contact;
    this.currentGroup = null;

    const headerName = document.getElementById("chat-header-name");
    const headerAvatar = document.getElementById("chat-header-avatar");
    if (headerName) headerName.textContent = contact.display_name;
    if (headerAvatar) {
      headerAvatar.textContent = contact.display_name.charAt(0).toUpperCase();
      headerAvatar.className = `avatar ${this.avatarColor(contact.display_name)}`;
    }

    this.showScreen("chat");
    await this.loadMessages(contact.onion_address);
    this.markAsRead(contact.onion_address);
    this.scrollToBottom();
  }

  async openGroupChat(group: Group): Promise<void> {
    this.currentGroup = group;
    this.currentContact = null;

    const headerName = document.getElementById("group-chat-header-name");
    const headerInfo = document.getElementById("group-chat-header-info");
    if (headerName) headerName.textContent = group.name;
    if (headerInfo) headerInfo.textContent = `${group.member_count} members`;

    this.showScreen("group-chat");
    await this.loadGroupMessages(group.id);
    this.scrollToBottom();
  }

  setupChatView(): void {
    document.getElementById("chat-back-btn")?.addEventListener("click", () => {
      this.showScreen("chats");
      this.currentContact = null;
    });

    document.getElementById("chat-call-btn")?.addEventListener("click", () => {
      this.startCall(false);
    });

    document.getElementById("chat-video-btn")?.addEventListener("click", () => {
      this.startCall(true);
    });

    document.getElementById("chat-send-btn")?.addEventListener("click", () => {
      this.sendMessage();
    });

    document.getElementById("chat-message-input")?.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        this.sendMessage();
      }
    });

    const msgInput = document.getElementById("chat-message-input") as HTMLTextAreaElement | null;
    if (msgInput) {
      msgInput.addEventListener("input", () => {
        msgInput.style.height = "auto";
        msgInput.style.height = Math.min(msgInput.scrollHeight, 120) + "px";
        this.sendTyping(true);
      });
    }

    document.getElementById("chat-attach-btn")?.addEventListener("click", () => {
      this.showModal("file-picker-modal");
    });

    document.getElementById("chat-emoji-btn")?.addEventListener("click", () => {
      const picker = document.getElementById("emoji-picker");
      if (picker) picker.classList.toggle("hidden");
    });

    document.getElementById("chat-voice-btn")?.addEventListener("click", async () => {
      if (this.mediaRecorder && this.mediaRecorder.state === "recording") {
        this.mediaRecorder.stop();
        return;
      }
      
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        this.mediaRecorder = new MediaRecorder(stream);
        this.audioChunks = [];
        this.recordingStartTime = Date.now();
        
        const voiceBtn = document.getElementById("chat-voice-btn");
        if (voiceBtn) voiceBtn.classList.add("recording");
        
        this.mediaRecorder.ondataavailable = (e) => {
          if (e.data.size > 0) this.audioChunks.push(e.data);
        };
        
        this.mediaRecorder.onstop = async () => {
          stream.getTracks().forEach(t => t.stop());
          if (voiceBtn) voiceBtn.classList.remove("recording");
          
          const duration = (Date.now() - this.recordingStartTime) / 1000;
          const blob = new Blob(this.audioChunks, { type: "audio/webm" });
          
          const reader = new FileReader();
          reader.onload = async () => {
            try {
              const base64 = (reader.result as string).split(",")[1] || "";
              if (this.currentContact) {
                await api.sendVoiceNote(
                  this.currentContact.onion_address,
                  base64,
                  `voice-${Date.now()}.webm`,
                  "audio/webm",
                  duration
                );
                await this.loadMessages(this.currentContact.onion_address);
                this.scrollToBottom();
                this.showToast("Voice note sent");
              }
            } catch (err) {
              console.error("Failed to send voice note:", err);
              this.showToast("Failed to send voice note");
            }
          };
          reader.readAsDataURL(blob);
        };
        
        this.mediaRecorder.start();
        this.showToast("Recording... tap mic to stop");
      } catch (err) {
        this.showToast("Microphone access denied");
      }
    });

    document.getElementById("chat-header-area")?.addEventListener("click", () => {
      if (this.currentContact) this.showContactInfo(this.currentContact);
    });
  }

  setupGroupChatView(): void {
    document.getElementById("group-chat-back-btn")?.addEventListener("click", () => {
      this.showScreen("chats");
      this.currentGroup = null;
    });

    document.getElementById("group-chat-send-btn")?.addEventListener("click", () => {
      this.sendGroupMessage();
    });

    document.getElementById("group-chat-message-input")?.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        this.sendGroupMessage();
      }
    });

    const msgInput = document.getElementById("group-chat-message-input") as HTMLTextAreaElement | null;
    if (msgInput) {
      msgInput.addEventListener("input", () => {
        msgInput.style.height = "auto";
        msgInput.style.height = Math.min(msgInput.scrollHeight, 120) + "px";
      });
    }

    document.getElementById("group-chat-attach-btn")?.addEventListener("click", () => {
      this.showModal("file-picker-modal");
    });

    document.getElementById("group-chat-emoji-btn")?.addEventListener("click", () => {
      const picker = document.getElementById("emoji-picker-group");
      if (picker) picker.classList.toggle("hidden");
    });

    document.getElementById("group-chat-info-btn")?.addEventListener("click", () => {
      if (this.currentGroup) this.showGroupInfo(this.currentGroup);
    });
  }

  async loadMessages(contactOnion: string): Promise<void> {
    try {
      const messages = await api.getMessages(contactOnion);
      store.setMessagesForContact(contactOnion, messages);
      this.renderMessages(messages);
      this.setupReplyPreview();
    } catch (err) {
      console.error("Failed to load messages:", err);
      this.showToast("Failed to load messages");
    }
  }

  async loadGroupMessages(groupId: string): Promise<void> {
    try {
      const messages = await api.getGroupMessages(groupId);
      store.setGroupMessagesForGroup(groupId, messages);
      this.renderGroupMessages(messages);
    } catch (err) {
      console.error("Failed to load group messages:", err);
      this.showToast("Failed to load messages");
    }
  }

  async renderMessages(messages: Message[]): Promise<void> {
    const container = document.getElementById("chat-messages");
    if (!container) return;

    if (messages.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">🔒</div>
          <h3>End-to-end encrypted</h3>
          <p>Send a message to start the conversation</p>
        </div>`;
      return;
    }

    for (const msg of messages) {
      try {
        const reactions = await api.getReactions(msg.id);
        store.setReactionsForMessage(msg.id, reactions);
      } catch { /* silent */ }
    }

    const identity = store.getIdentity();
    let html = "";
    let lastDate = "";

    for (const msg of messages) {
      const dateStr = this.formatDate(msg.timestamp);
      if (dateStr !== lastDate) {
        html += `<div class="date-separator"><span>${dateStr}</span></div>`;
        lastDate = dateStr;
      }

      const isSent = msg.sender === identity?.onion_address;
      const statusIcon = isSent
        ? msg.read
          ? '<span class="msg-status msg-read">✓✓</span>'
          : msg.delivered
          ? '<span class="msg-status msg-delivered">✓✓</span>'
          : '<span class="msg-status msg-sent">✓</span>'
        : "";

      const lockIcon = '<span class="msg-lock">🔒</span>';
      const expiryIcon = msg.expires_at ? '<span class="msg-expiry">⏱</span>' : "";

      let replyPreview = "";
      if (msg.reply_to) {
        const replyMsg = messages.find((m) => m.id === msg.reply_to);
        replyPreview = `
          <div class="reply-preview">
            <div class="reply-preview-text">${replyMsg ? this.esc(replyMsg.content || "Attachment") : "Message"}</div>
          </div>`;
      }

      let reactionsHtml = "";
      const reactionsForMsg = store.getReactionsForMessage(msg.id);
      if (reactionsForMsg.length > 0) {
        const reactionMap = new Map<string, number>();
        reactionsForMsg.forEach((r: Reaction) => {
          reactionMap.set(r.emoji, (reactionMap.get(r.emoji) || 0) + 1);
        });
        const items = Array.from(reactionMap.entries())
          .map(([emoji, count]) => `<span class="reaction-chip">${emoji} ${count > 1 ? count : ""}</span>`)
          .join("");
        reactionsHtml = `<div class="message-reactions">${items}</div>`;
      }

      const bodyHtml = msg.content ? `<div class="msg-text">${this.esc(msg.content)}</div>` : "";
      const attachmentHtml = msg.message_type === "file" || msg.message_type === "image"
        ? `<div class="msg-attachment">📎 ${this.esc(msg.content || "File")}</div>`
        : "";

      html += `
        <div class="message ${isSent ? "sent" : "received"}" data-msg-id="${this.esc(msg.id)}">
          ${replyPreview}
          <div class="msg-bubble">
            ${bodyHtml}
            ${attachmentHtml}
            <div class="msg-footer">
              ${lockIcon}
              <span class="msg-time">${this.formatTime(msg.timestamp)}</span>
              ${expiryIcon}
              ${statusIcon}
            </div>
          </div>
          ${reactionsHtml}
        </div>`;
    }

    container.innerHTML = html;

    container.querySelectorAll(".message").forEach((el) => {
      const msgId = el.getAttribute("data-msg-id");
      const msg = messages.find((m) => m.id === msgId);
      if (!msg) return;

      let pressTimer: any;
      el.addEventListener("mousedown", () => {
        pressTimer = setTimeout(() => {
          this.contextMenuTarget = msg;
          this.showContextMenu(el as HTMLElement);
        }, 500);
      });
      el.addEventListener("mouseup", () => clearTimeout(pressTimer));
      el.addEventListener("mouseleave", () => clearTimeout(pressTimer));

      el.addEventListener("touchstart", () => {
        pressTimer = setTimeout(() => {
          this.contextMenuTarget = msg;
          this.showContextMenu(el as HTMLElement);
        }, 500);
      });
      el.addEventListener("touchend", () => clearTimeout(pressTimer));

      el.addEventListener("dblclick", async () => {
        try {
          await api.addReaction(msg.id, "❤️");
          const reactions = await api.getReactions(msg.id);
          store.setReactionsForMessage(msg.id, reactions);
          this.renderMessages(messages);
        } catch { /* silent */ }
      });
    });
  }

  renderGroupMessages(messages: GroupMessage[]): void {
    const container = document.getElementById("group-chat-messages");
    if (!container) return;

    if (messages.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-icon">🔒</div>
          <h3>End-to-end encrypted</h3>
          <p>Send a message to start the conversation</p>
        </div>`;
      return;
    }

    const identity = store.getIdentity();
    let html = "";
    let lastDate = "";

    for (const msg of messages) {
      const dateStr = this.formatDate(msg.timestamp);
      if (dateStr !== lastDate) {
        html += `<div class="date-separator"><span>${dateStr}</span></div>`;
        lastDate = dateStr;
      }

      const isSent = msg.sender === identity?.onion_address;
      const senderName = isSent ? "You" : this.esc(msg.sender.slice(0, 12));

      let replyPreview = "";
      if (msg.reply_to) {
        replyPreview = `
          <div class="reply-preview">
            <div class="reply-preview-text">Reply</div>
          </div>`;
      }

      const bodyHtml = msg.content ? `<div class="msg-text">${this.esc(msg.content)}</div>` : "";

      html += `
        <div class="message ${isSent ? "sent" : "received"}" data-msg-id="${this.esc(msg.id)}">
          ${!isSent ? `<div class="msg-sender">${senderName}</div>` : ""}
          ${replyPreview}
          <div class="msg-bubble">
            ${bodyHtml}
            <div class="msg-footer">
              <span class="msg-lock">🔒</span>
              <span class="msg-time">${this.formatTime(msg.timestamp)}</span>
            </div>
          </div>
        </div>`;
    }

    container.innerHTML = html;
  }

  async sendMessage(): Promise<void> {
    if (!this.currentContact) return;
    const input = document.getElementById("chat-message-input") as HTMLTextAreaElement | null;
    const text = input?.value.trim();
    if (!text && !this.replyToMessage) return;

    try {
      let msg: Message;
      if (this.replyToMessage) {
        msg = await api.sendReplyMessage(this.currentContact.onion_address, text || "", "text", this.replyToMessage.id);
      } else {
        msg = await api.sendMessage(this.currentContact.onion_address, text || "", "text");
      }
      store.addMessage(this.currentContact.onion_address, msg);
      this.renderMessages(store.getMessagesForContact(this.currentContact.onion_address));
      if (input) {
        input.value = "";
        input.style.height = "auto";
      }
      this.replyToMessage = null;
      this.clearReplyPreview();
      this.scrollToBottom();
    } catch (err) {
      console.error("Failed to send message:", err);
      this.showToast("Failed to send message");
    }
  }

  async sendGroupMessage(): Promise<void> {
    if (!this.currentGroup) return;
    const input = document.getElementById("group-chat-message-input") as HTMLTextAreaElement | null;
    const text = input?.value.trim();
    if (!text) return;

    try {
      const msg = await api.sendGroupMessage(this.currentGroup.id, text, "text");
      store.addGroupMessage(this.currentGroup.id, msg);
      this.renderGroupMessages(store.getGroupMessagesForGroup(this.currentGroup.id));
      if (input) {
        input.value = "";
        input.style.height = "auto";
      }
      this.scrollToBottom();
    } catch (err) {
      console.error("Failed to send group message:", err);
      this.showToast("Failed to send message");
    }
  }

  setupModals(): void {
    document.getElementById("add-contact-open")?.addEventListener("click", () => {
      this.showModal("add-contact-modal");
    });

    document.getElementById("add-contact-close")?.addEventListener("click", () => {
      this.hideModal("add-contact-modal");
    });

    document.getElementById("add-contact-save")?.addEventListener("click", async () => {
      const nameInput = document.getElementById("add-contact-name") as HTMLInputElement | null;
      const onionInput = document.getElementById("add-contact-onion") as HTMLInputElement | null;
      const pubkeyInput = document.getElementById("add-contact-pubkey") as HTMLInputElement | null;
      const name = nameInput?.value.trim();
      const onion = onionInput?.value.trim();
      const pubkey = pubkeyInput?.value.trim() || "";
      if (!name || !onion) {
        this.showToast("Name and onion address required");
        return;
      }
      try {
        await api.addContact(name, pubkey, onion);
        await this.loadContacts();
        this.hideModal("add-contact-modal");
        if (nameInput) nameInput.value = "";
        if (onionInput) onionInput.value = "";
        if (pubkeyInput) pubkeyInput.value = "";
        this.showToast("Contact added");
      } catch (err) {
        console.error("Failed to add contact:", err);
        this.showToast("Failed to add contact");
      }
    });

    document.getElementById("qr-code-open")?.addEventListener("click", async () => {
      this.showModal("qr-code-modal");
      const qrImg = document.getElementById("qr-code-image") as HTMLImageElement | null;
      const identity = store.getIdentity();
      if (qrImg && identity) {
        try {
          const dataUrl = await api.generateQrCode();
          qrImg.src = dataUrl;
        } catch (err) {
          console.error("Failed to generate QR:", err);
        }
      }
    });

    document.getElementById("qr-code-close")?.addEventListener("click", () => {
      this.hideModal("qr-code-modal");
    });

    document.getElementById("qr-scan-camera")?.addEventListener("click", async () => {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
        const video = document.createElement("video");
        video.srcObject = stream;
        video.setAttribute("playsinline", "");
        video.setAttribute("muted", "");
        video.play();

        const scanDiv = document.createElement("div");
        scanDiv.className = "qr-scan-overlay";
        scanDiv.innerHTML = '<p>Point camera at QR code</p>';
        scanDiv.appendChild(video);

        const statusEl = document.createElement("p");
        statusEl.className = "qr-scan-status";
        statusEl.textContent = "Scanning...";
        scanDiv.appendChild(statusEl);

        const cancelBtn = document.createElement("button");
        cancelBtn.textContent = "Cancel";
        cancelBtn.className = "btn btn-secondary";
        scanDiv.appendChild(cancelBtn);
        document.body.appendChild(scanDiv);

        let done = false;
        const stopScan = () => {
          if (done) return;
          done = true;
          stream.getTracks().forEach(t => t.stop());
          scanDiv.remove();
        };
        cancelBtn.addEventListener("click", stopScan);

        const canvas = document.createElement("canvas");
        const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
        let frameCount = 0;

        const decodeFrame = async () => {
          if (done || !scanDiv.parentNode) return;
          try {
            canvas.width = video.videoWidth;
            canvas.height = video.videoHeight;
            if (canvas.width > 0 && canvas.height > 0) {
              ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
              const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
              const code = jsQR(imageData.data, imageData.width, imageData.height, {
                inversionAttempts: "dontInvert",
              });
              if (code && code.data && code.data.trim()) {
                statusEl.textContent = "QR detected! Verifying...";
                try {
                  await api.scanQrCode(code.data.trim());
                  await this.loadContacts();
                  this.showToast("Contact added from QR");
                  this.hideModal("qr-code-modal");
                  stopScan();
                  return;
                } catch (err) {
                  statusEl.textContent = "Invalid QR code — keep trying or paste data below";
                }
              } else {
                statusEl.textContent = "Scanning...";
              }
            }
          } catch {
            // frame decode errors are transient
          }

          frameCount++;
          // Throttle detection to every 3rd frame to save CPU
          if (frameCount % 3 === 0) {
            setTimeout(decodeFrame, 100);
          } else {
            requestAnimationFrame(decodeFrame);
          }
        };
        video.addEventListener("playing", () => decodeFrame());

        // Also allow paste
        const pasteInput = document.createElement("input");
        pasteInput.placeholder = "Or paste QR data here";
        pasteInput.className = "input";
        pasteInput.style.marginTop = "8px";
        pasteInput.addEventListener("paste", async () => {
          setTimeout(async () => {
            const data = pasteInput.value.trim();
            if (data) {
              try {
                await api.scanQrCode(data);
                await this.loadContacts();
                this.showToast("Contact added from QR");
                this.hideModal("qr-code-modal");
                stopScan();
              } catch (err) {
                this.showToast("Invalid QR code");
              }
            }
          }, 100);
        });
        scanDiv.appendChild(pasteInput);

      } catch (err) {
        console.error("Camera access failed:", err);
        this.showToast("Camera access denied");
      }
    });

    document.getElementById("qr-scan-file")?.addEventListener("click", () => {
      const fileInput = document.createElement("input");
      fileInput.type = "file";
      fileInput.accept = "image/*";
      fileInput.addEventListener("change", async () => {
        const file = fileInput.files?.[0];
        if (!file) return;

        const processImage = async (img: HTMLImageElement) => {
          try {
            const canvas = document.createElement("canvas");
            canvas.width = img.naturalWidth;
            canvas.height = img.naturalHeight;
            const ctx = canvas.getContext("2d", { willReadFrequently: true });
            if (!ctx) throw new Error("no-canvas");
            ctx.drawImage(img, 0, 0);
            const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);

            // Try both normal and inverted orientations for robustness
            let code = jsQR(imageData.data, imageData.width, imageData.height, {
              inversionAttempts: "attemptBoth",
            });
            if (code && code.data && code.data.trim()) {
              try {
                await api.scanQrCode(code.data.trim());
                await this.loadContacts();
                this.showToast("Contact added from QR");
                this.hideModal("qr-code-modal");
                return;
              } catch (err) {
                this.showToast("Invalid QR code in image");
                return;
              }
            }
            this.showToast("No QR code found in image");
          } catch (err) {
            console.error("File QR decode failed:", err);
            this.showToast("Could not decode image");
          }
        };

        const reader = new FileReader();
        reader.onload = () => {
          const img = new Image();
          img.onload = () => processImage(img);
          img.onerror = () => this.showToast("Could not read image");
          img.src = reader.result as string;
        };
        reader.readAsDataURL(file);
      });
      fileInput.click();
    });

    document.getElementById("edit-name-open")?.addEventListener("click", () => {
      const input = document.getElementById("edit-name-input") as HTMLInputElement | null;
      const identity = store.getIdentity();
      if (input && identity) input.value = identity.display_name;
      this.showModal("edit-name-modal");
    });

    document.getElementById("edit-name-close")?.addEventListener("click", () => {
      this.hideModal("edit-name-modal");
    });

    document.getElementById("edit-name-save")?.addEventListener("click", async () => {
      const input = document.getElementById("edit-name-input") as HTMLInputElement | null;
      const name = input?.value.trim();
      if (!name) {
        this.showToast("Name is required");
        return;
      }
      try {
        await api.initIdentity(name);
        await this.loadIdentity();
        this.hideModal("edit-name-modal");
        this.showToast("Name updated");
      } catch (err) {
        this.showToast("Failed to update name");
      }
    });

    document.getElementById("create-group-open")?.addEventListener("click", () => {
      this.showModal("create-group-modal");
    });

    document.getElementById("create-group-close")?.addEventListener("click", () => {
      this.hideModal("create-group-modal");
    });

    document.getElementById("create-group-save")?.addEventListener("click", async () => {
      const nameInput = document.getElementById("create-group-name") as HTMLInputElement | null;
      const name = nameInput?.value.trim();
      if (!name) {
        this.showToast("Group name required");
        return;
      }
      try {
        await api.createGroup(name, name);
        await this.loadGroups();
        this.hideModal("create-group-modal");
        if (nameInput) nameInput.value = "";
        this.showToast("Group created");
      } catch (err) {
        this.showToast("Failed to create group");
      }
    });

    document.getElementById("contact-info-close")?.addEventListener("click", () => {
      this.hideModal("contact-info-modal");
    });

    document.getElementById("group-info-close")?.addEventListener("click", () => {
      this.hideModal("group-info-modal");
    });

    document.getElementById("group-add-member-btn")?.addEventListener("click", async () => {
      const input = document.getElementById("group-add-member-input") as HTMLInputElement | null;
      const onion = input?.value.trim();
      if (!onion || !this.currentGroup) return;
      try {
        const identity = store.getIdentity();
        await api.addGroupMember(this.currentGroup.id, identity?.display_name || "Member", "", onion);
        await this.loadGroups();
        if (input) input.value = "";
        this.showToast("Member added");
      } catch (err) {
        this.showToast("Failed to add member");
      }
    });

    document.getElementById("delete-data-confirm")?.addEventListener("click", async () => {
      try {
        await api.deleteAllData();
        this.hideModal("delete-data-modal");
        store.clearAll();
        this.showToast("All data deleted");
        window.location.reload();
      } catch (err) {
        this.showToast("Failed to delete data");
      }
    });

    document.getElementById("delete-data-open")?.addEventListener("click", () => {
      this.showModal("delete-data-modal");
    });

    document.getElementById("delete-data-cancel")?.addEventListener("click", () => {
      this.hideModal("delete-data-modal");
    });

    document.getElementById("file-picker-send")?.addEventListener("click", async () => {
      const input = document.getElementById("file-picker-input") as HTMLInputElement | null;
      const file = input?.files?.[0];
      if (file) {
        const reader = new FileReader();
        reader.onload = async () => {
          try {
            const base64 = (reader.result as string).split(",")[1] || "";
            await this.handleFileSend(base64, file.name, file.type);
            this.hideModal("file-picker-modal");
          } catch (err) {
            this.showToast("Failed to send file");
          }
        };
        reader.readAsDataURL(file);
      }
    });

    document.getElementById("file-picker-cancel")?.addEventListener("click", () => {
      this.hideModal("file-picker-modal");
    });

    document.querySelectorAll(".modal-overlay").forEach((overlay) => {
      overlay.addEventListener("click", (e) => {
        if (e.target === overlay) {
          (overlay as HTMLElement).classList.add("hidden");
        }
      });
    });
  }

  setupCallOverlay(): void {
    document.getElementById("call-mute-btn")?.addEventListener("click", () => {
      const btn = document.getElementById("call-mute-btn");
      btn?.classList.toggle("active");
      if (this.activeCallId) api.toggleAudioMute(this.activeCallId);
    });

    document.getElementById("call-speaker-btn")?.addEventListener("click", () => {
      const btn = document.getElementById("call-speaker-btn");
      btn?.classList.toggle("active");
    });

    document.getElementById("call-end-btn")?.addEventListener("click", () => {
      this.endCall();
    });

    document.getElementById("call-video-btn")?.addEventListener("click", () => {
      const btn = document.getElementById("call-video-btn");
      btn?.classList.toggle("active");
      if (this.activeCallId) api.toggleVideo(this.activeCallId);
    });

    document.getElementById("call-screen-btn")?.addEventListener("click", () => {
      const btn = document.getElementById("call-screen-btn");
      const active = btn?.classList.toggle("active");
      if (!this.activeCallId) return;
      if (active) {
        this.startScreenCapturing();
      } else {
        this.stopScreenCapturing();
      }
    });

    document.getElementById("call-answer-btn")?.addEventListener("click", () => {
      this.answerIncomingCall();
    });

    document.getElementById("call-decline-btn")?.addEventListener("click", () => {
      this.declineIncomingCall();
    });
  }

  async startCall(video: boolean): Promise<void> {
    if (!this.currentContact) return;
    try {
      let callId: string;
      if (video) {
        callId = await api.startVideoCall(this.currentContact.onion_address);
      } else {
        callId = await api.startAudioCall(this.currentContact.onion_address);
      }
      this.activeCallId = callId;
      this.callSeconds = 0;
      this.isIncomingCall = false;
      this.incomingPeerOnion = null;
      this.activeCallPeerOnion = this.currentContact.onion_address;
      this.activeCallIsVideo = video;

      const overlay = document.getElementById("call-overlay");
      overlay?.classList.remove("hidden");

      const statusEl = document.getElementById("call-status");
      if (statusEl) statusEl.textContent = "Ringing...";

      const nameEl = document.getElementById("call-name");
      if (nameEl) nameEl.textContent = this.currentContact.display_name;

      const incomingCtrls = document.getElementById("call-incoming-controls");
      incomingCtrls?.classList.add("hidden");

      this.callTimerInterval = setInterval(() => {
        this.callSeconds++;
        const timerEl = document.getElementById("call-timer");
        if (timerEl) timerEl.textContent = this.formatDuration(this.callSeconds);
      }, 1000);

      this.showToast(`Calling ${this.currentContact.display_name}...`);
    } catch (err) {
      console.error("Failed to start call:", err);
      this.showToast("Failed to start call");
    }
  }

  async answerIncomingCall(): Promise<void> {
    if (!this.activeCallId || !this.incomingPeerOnion) return;
    try {
      await api.answerVideoCall(this.activeCallId);
      const statusEl = document.getElementById("call-status");
      if (statusEl) statusEl.textContent = "Connected";
      const incomingCtrls = document.getElementById("call-incoming-controls");
      incomingCtrls?.classList.add("hidden");
      this.showToast("Call connected");
      this.startCallMedia(this.activeCallIsVideo);
    } catch (err) {
      console.error("Failed to answer call:", err);
      this.showToast("Failed to answer call");
    }
  }

  async declineIncomingCall(): Promise<void> {
    const callId = this.activeCallId;
    const peer = this.incomingPeerOnion;
    this.endCall();
    if (callId && peer) {
      try { await api.rejectCall(callId); } catch { /* ignore */ }
    }
    this.showToast("Call declined");
  }

  async startCallMedia(video: boolean): Promise<void> {
    this.callSeq = 0;
    // Capture local media (mic, and camera if video)
    try {
      const constraints: MediaStreamConstraints = { audio: true };
      if (video) constraints.video = { width: { ideal: 320 }, height: { ideal: 240 }, facingMode: "user" };
      this.callMediaStream = await navigator.mediaDevices.getUserMedia(constraints);
    } catch (err) {
      console.warn("Media capture partly failed:", err);
      this.showToast("Microphone/camera not available");
      return;
    }

    // Show video container for video calls
    const vidContainer = document.getElementById("call-video-container");
    if (vidContainer) {
      if (video) {
        vidContainer.classList.remove("hidden");
        this.renderLocalVideo();
      } else {
        vidContainer.classList.add("hidden");
      }
    }

    // Start sending the remote's frames timer (video) and audio packets
    this.startVoiceStreaming();

    if (video) {
      this.startVideoStreaming();
    }
  }

  startVoiceStreaming(): void {
    this.startMicVoiceStream();
  }

  startMicVoiceStream(): void {
    const stream = this.callMediaStream;
    if (!stream || !this.activeCallId || !this.currentContact) return;
    const micTracks = stream.getAudioTracks();
    if (micTracks.length === 0) return;
    const micStream = new MediaStream(micTracks);
    this.mediaRecorder = new MediaRecorder(micStream, { mimeType: "audio/mp4" });
    this.mediaRecorder.ondataavailable = (e) => {
      if (e.data && e.data.size > 0) this.sendVoiceChunk(e.data);
    };
    this.mediaRecorder.start(500);
  }

  sendVoiceChunk(blob: Blob): void {
    const callId = this.activeCallId;
    const peer = this.activeCallPeerOnion;
    if (!callId || !peer) return;
    blob.arrayBuffer().then((buf) => {
      const data = Array.from(new Uint8Array(buf));
      const seq = this.callSeq++;
      // Packetize in 500ms chunks -> send in ~8KB pieces over the wire
      const chunkSize = 8000;
      for (let i = 0; i < data.length; i += chunkSize) {
        const piece = data.slice(i, i + chunkSize);
        api.sendVoicePacket(peer, callId, seq, piece, 48000, 1).catch(() => {});
      }
    }).catch(() => {});
  }

  async startVideoStreaming(): Promise<void> {
    this.stopVideoStreaming();
    this.callVideoTimer = setInterval(() => {
      this.captureAndSendFrame();
    }, 500);
  }

  captureAndSendFrame(): void {
    const callId = this.activeCallId;
    const peer = this.activeCallPeerOnion;
    const stream = this.callMediaStream;
    if (!callId || !peer || !stream) return;
    const videoTracks = stream.getVideoTracks();
    if (videoTracks.length === 0) return;

    const vidEl = document.createElement("video");
    vidEl.srcObject = stream;
    vidEl.muted = true;
    vidEl.onloadedmetadata = () => {
      vidEl.play();
      const canvas = document.createElement("canvas");
      canvas.width = 160;
      canvas.height = 120;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.drawImage(vidEl, 0, 0, 160, 120);
      const jpeg = canvas.toDataURL("image/jpeg", 0.4);
      const base64 = jpeg.split(",")[1];
      if (!base64) return;
      try {
        const binary = atob(base64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        api.sendVideoFrame(peer, callId, this.callSeq++, Array.from(bytes), 160, 120).catch(() => {});
        // Mirror to local preview
        const localCtx = (document.getElementById("call-local-video") as HTMLCanvasElement | null)?.getContext("2d");
        if (localCtx) localCtx.drawImage(vidEl, 0, 0, 160, 120);
      } catch { /* ignore */ }
    };
  }

  async startScreenCapturing(): Promise<void> {
    try {
      this.stopScreenCapturing();
      const screenStream = await (navigator.mediaDevices as any).getDisplayMedia({ video: true });
      this.screenStream = screenStream;
      this.stopVideoStreaming();
      this.callVideoTimer = setInterval(() => {
        const callId = this.activeCallId;
        const peer = this.activeCallPeerOnion;
        if (!callId || !peer) return;
        const vidEl = document.createElement("video");
        vidEl.srcObject = this.screenStream;
        vidEl.muted = true;
        vidEl.onloadedmetadata = () => {
          vidEl.play();
          const canvas = document.createElement("canvas");
          canvas.width = 200;
          canvas.height = 150;
          const ctx = canvas.getContext("2d");
          if (!ctx) return;
          ctx.drawImage(vidEl, 0, 0, 200, 150);
          const jpeg = canvas.toDataURL("image/jpeg", 0.3);
          const base64 = jpeg.split(",")[1];
          if (!base64) return;
          try {
            const binary = atob(base64);
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
            api.sendScreenFrame(peer, callId, this.callSeq++, Array.from(bytes), 200, 150).catch(() => {});
          } catch { }
        };
      }, 700);
    } catch (err) {
      this.showToast("Screen share not available");
    }
  }

  stopScreenCapturing(): void {
    this.stopVideoStreaming();
    if (this.screenStream) {
      this.screenStream.getTracks().forEach(t => t.stop());
      this.screenStream = null;
    }
    const btn = document.getElementById("call-screen-btn");
    btn?.classList.remove("active");
  }

  stopVideoStreaming(): void {
    if (this.callVideoTimer) {
      clearInterval(this.callVideoTimer);
      this.callVideoTimer = null;
    }
  }

  async endCall(): Promise<void> {
    const callId = this.activeCallId;
    if (callId) {
      try {
        await api.endVideoCall(callId);
      } catch (err) {
        console.error("Failed to end call:", err);
      }
    }

    // Stop media resources
    this.callMediaStream?.getTracks().forEach(t => t.stop());
    this.callMediaStream = null;
    this.stopScreenCapturing();
    this.stopVideoStreaming();
    if (this.mediaRecorder) {
      try { this.mediaRecorder.stop(); } catch { /* ignore */ }
      this.mediaRecorder = null;
    }

    this.activeCallId = null;
    this.isIncomingCall = false;
    this.incomingPeerOnion = null;
    this.activeCallPeerOnion = null;
    this.activeCallIsVideo = false;
    this.callSeq = 0;
    this.voiceQ = [];
    this.voicePlaying = false;
    clearInterval(this.callTimerInterval);
    this.callTimerInterval = null;
    this.callSeconds = 0;

    const overlay = document.getElementById("call-overlay");
    overlay?.classList.add("hidden");
    const vidContainer = document.getElementById("call-video-container");
    vidContainer?.classList.add("hidden");
    const remoteC = document.getElementById("call-remote-container");
    if (remoteC) {
      const img = remoteC.querySelector("img");
      if (img) img.remove();
    }

    await this.loadCallHistory();
  }

  renderLocalVideo(): void {
    const vidContainer = document.getElementById("call-video-container");
    if (vidContainer) vidContainer.classList.remove("hidden");
  }

  setupContextMenu(): void {
    document.getElementById("ctx-reply")?.addEventListener("click", () => {
      if (this.contextMenuTarget) {
        this.replyToMessage = this.contextMenuTarget;
        this.showReplyPreview(this.contextMenuTarget);
        const input = document.getElementById("chat-message-input") as HTMLTextAreaElement | null;
        input?.focus();
      }
      this.hideContextMenu();
    });

    document.getElementById("ctx-copy")?.addEventListener("click", async () => {
      if (this.contextMenuTarget?.content) {
        try {
          await navigator.clipboard.writeText(this.contextMenuTarget.content);
          this.showToast("Copied to clipboard");
        } catch {
          this.showToast("Failed to copy");
        }
      }
      this.hideContextMenu();
    });

    document.getElementById("ctx-delete")?.addEventListener("click", async () => {
      if (this.contextMenuTarget && this.currentContact) {
        try {
          await api.deleteMessage(this.contextMenuTarget.id);
          store.removeMessage(this.contextMenuTarget.id);
          this.renderMessages(store.getMessagesForContact(this.currentContact.onion_address));
          this.showToast("Message deleted");
        } catch (err) {
          this.showToast("Failed to delete message");
        }
      }
      this.hideContextMenu();
    });

    document.getElementById("ctx-forward")?.addEventListener("click", () => {
      if (!this.contextMenuTarget) { this.hideContextMenu(); return; }

      const contacts = store.getContacts();
      if (contacts.length === 0) {
        this.showToast("No contacts to forward to");
        this.hideContextMenu();
        return;
      }

      const forwardDiv = document.createElement("div");
      forwardDiv.className = "modal-overlay";
      forwardDiv.innerHTML = `
        <div class="modal">
          <div class="modal-header">
            <h3>Forward message</h3>
            <button class="modal-close">&times;</button>
          </div>
          <div class="modal-body">
            <div class="forward-contact-list">
              ${contacts.map(c => `
                <div class="forward-contact-item" data-onion="${this.esc(c.onion_address)}">
                  <div class="avatar ${this.avatarColor(c.display_name)}">${c.display_name.charAt(0).toUpperCase()}</div>
                  <span>${this.esc(c.display_name)}</span>
                </div>
              `).join("")}
            </div>
          </div>
        </div>`;

      forwardDiv.querySelector(".modal-close")?.addEventListener("click", () => forwardDiv.remove());
      forwardDiv.addEventListener("click", (e) => { if (e.target === forwardDiv) forwardDiv.remove(); });

      const msgToForward = this.contextMenuTarget;
      forwardDiv.querySelectorAll(".forward-contact-item").forEach(el => {
        el.addEventListener("click", async () => {
          const onion = el.getAttribute("data-onion");
          if (!onion || !msgToForward) return;
          try {
            await api.sendForwardMessage(onion, msgToForward.sender, msgToForward.content || "");
            this.showToast("Message forwarded");
            forwardDiv.remove();
          } catch (err) {
            this.showToast("Failed to forward message");
          }
        });
      });

      document.body.appendChild(forwardDiv);
      this.hideContextMenu();
    });

    document.getElementById("ctx-disappear")?.addEventListener("click", async () => {
      if (this.contextMenuTarget) {
        try {
          await api.setDisappearingMessage(this.contextMenuTarget.id, 3600);
          this.showToast("Disappearing message set (1 hour)");
        } catch (err) {
          this.showToast("Failed to set disappearing message");
        }
      }
      this.hideContextMenu();
    });

    document.querySelectorAll(".ctx-reaction-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        if (this.contextMenuTarget) {
          const emoji = btn.getAttribute("data-emoji");
          if (emoji) {
            try {
              await api.addReaction(this.contextMenuTarget.id, emoji);
              if (this.currentContact) {
                const reactions = await api.getReactions(this.contextMenuTarget.id);
                store.setReactionsForMessage(this.contextMenuTarget.id, reactions);
                this.renderMessages(store.getMessagesForContact(this.currentContact.onion_address));
              }
            } catch { /* silent */ }
          }
        }
        this.hideContextMenu();
      });
    });

    document.addEventListener("click", (e) => {
      const menu = document.getElementById("context-menu");
      if (menu && !menu.contains(e.target as Node)) {
        this.hideContextMenu();
      }
    });
  }

  showContextMenu(target: HTMLElement): void {
    const menu = document.getElementById("context-menu");
    if (!menu) return;
    menu.classList.remove("hidden");
    const rect = target.getBoundingClientRect();
    menu.style.top = `${rect.top}px`;
    menu.style.left = `${rect.left}px`;
  }

  hideContextMenu(): void {
    document.getElementById("context-menu")?.classList.add("hidden");
  }

  setupEmojiPicker(): void {
    const emojis = [
      "😀","😂","😍","🥰","😊","😎","🤔","😢","😡","👍",
      "👎","❤️","🔥","🎉","👋","🙏","💪","🤝","👏","💯",
      "✅","❌","⭐","🌟","💡","🔒","📱","💻","🎵","🌈",
      "☀️","🌙","🌸","🍕","🍔","☕","🎂","🎮","⚽",
      "🏀","🎯","📚","✈️","🚗","🏠","🔑","💰","📞","📧"
    ];

    const picker = document.getElementById("emoji-picker-grid");
    if (picker) {
      picker.innerHTML = emojis.map((e) => `<span class="emoji-item">${e}</span>`).join("");
      picker.querySelectorAll(".emoji-item").forEach((el) => {
        el.addEventListener("click", () => {
          const input = document.getElementById("chat-message-input") as HTMLTextAreaElement | null;
          if (input) {
            const start = input.selectionStart;
            const end = input.selectionEnd;
            input.value = input.value.substring(0, start) + el.textContent + input.value.substring(end);
            input.selectionStart = input.selectionEnd = start + (el.textContent?.length || 0);
            input.focus();
          }
          picker.parentElement?.classList.add("hidden");
        });
      });
    }

    const groupPicker = document.getElementById("emoji-picker-group-grid");
    if (groupPicker) {
      groupPicker.innerHTML = emojis.map((e) => `<span class="emoji-item">${e}</span>`).join("");
      groupPicker.querySelectorAll(".emoji-item").forEach((el) => {
        el.addEventListener("click", () => {
          const input = document.getElementById("group-chat-message-input") as HTMLTextAreaElement | null;
          if (input) {
            const start = input.selectionStart;
            const end = input.selectionEnd;
            input.value = input.value.substring(0, start) + el.textContent + input.value.substring(end);
            input.selectionStart = input.selectionEnd = start + (el.textContent?.length || 0);
            input.focus();
          }
          el.closest(".emoji-picker")?.classList.add("hidden");
        });
      });
    }
  }

  async handleFileSend(fileData: string, fileName: string, mimeType: string): Promise<void> {
    if (!this.currentContact && !this.currentGroup) return;
    try {
      const progressEl = document.getElementById("file-send-progress");
      if (progressEl) progressEl.classList.remove("hidden");

      if (this.currentContact) {
        await api.sendFile(this.currentContact.onion_address, fileData, fileName, mimeType);
      } else if (this.currentGroup) {
        const members = await api.getGroupMembers(this.currentGroup.id);
        const identity = store.getIdentity();
        for (const m of members) {
          if (m.onion_address !== identity?.onion_address) {
            await api.sendFile(m.onion_address, fileData, fileName, mimeType);
          }
        }
      }
      this.showToast("File sent");
      if (progressEl) progressEl.classList.add("hidden");
    } catch (err) {
      console.error("Failed to send file:", err);
      this.showToast("Failed to send file");
    }
  }

  setupSearch(): void {
    const chatSearch = document.getElementById("chats-search-input") as HTMLInputElement | null;
    chatSearch?.addEventListener("input", () => {
      const query = chatSearch.value.toLowerCase();
      const contacts = store.getContacts();
      const filtered = contacts.filter(
        (c) => c.display_name.toLowerCase().includes(query) || c.onion_address.toLowerCase().includes(query)
      );
      this.renderChatList(filtered);
    });

    const contactSearch = document.getElementById("contacts-search-input") as HTMLInputElement | null;
    contactSearch?.addEventListener("input", () => {
      const query = contactSearch.value.toLowerCase();
      const contacts = store.getContacts();
      const filtered = contacts.filter(
        (c) => c.display_name.toLowerCase().includes(query) || c.onion_address.toLowerCase().includes(query)
      );
      this.renderContacts(filtered);
    });
  }

  setupEventListeners(): void {
    listen<Message>("new-message", (event) => {
      const msg = event.payload;
      const identity = store.getIdentity();
      const isFromMe = msg.sender === identity?.onion_address;
      const peerOnion = isFromMe ? msg.recipient : msg.sender;

      store.addMessage(peerOnion, msg);

      if (this.currentContact && (this.currentContact.onion_address === peerOnion)) {
        this.renderMessages(store.getMessagesForContact(peerOnion));
        this.scrollToBottom();
        this.markAsRead(peerOnion);
      }

      this.loadContacts();
    });

    listen("tor-status-changed", () => {
      this.updateTorStatus();
    });

    listen<{call_id: string, peer_onion: string, call_type: string}>("incoming-call", (event) => {
      const { call_id, peer_onion, call_type } = event.payload;
      this.activeCallId = call_id;
      this.isIncomingCall = true;
      this.incomingPeerOnion = peer_onion;
      this.activeCallPeerOnion = peer_onion;
      const isVideo = String(call_type).toLowerCase().startsWith("video");
      this.activeCallIsVideo = isVideo;
      this.callSeconds = 0;

      const overlay = document.getElementById("call-overlay");
      overlay?.classList.remove("hidden");

      const vidContainer = document.getElementById("call-video-container");
      vidContainer?.classList.add("hidden");

      const nameEl = document.getElementById("call-name");
      if (nameEl) nameEl.textContent = peer_onion.slice(0, 16);

      const statusEl = document.getElementById("call-status");
      if (statusEl) statusEl.textContent = "Incoming call...";

      const incomingCtrls = document.getElementById("call-incoming-controls");
      incomingCtrls?.classList.remove("hidden");

      // Start the incoming session so we can accept/decline over the wire
      api.createIncomingCall(call_id, peer_onion, isVideo ? "video" : "voice").catch((err) => {
        console.error("createIncomingCall failed:", err);
      });

      this.showToast(`Incoming ${call_type} call from ${peer_onion.slice(0, 16)}`);
    });

    listen("call-accepted", () => {
      const statusEl = document.getElementById("call-status");
      if (statusEl) statusEl.textContent = "Connected";
      const incomingCtrls = document.getElementById("call-incoming-controls");
      incomingCtrls?.classList.add("hidden");
      this.showToast("Call connected");
      this.isIncomingCall = false;
      this.incomingPeerOnion = null;
      // Once peer accepts, begin our own media send/receive
      this.startCallMedia(this.activeCallIsVideo);
    });

    listen("call-rejected", () => {
      this.showToast("Call rejected");
      this.endCall();
    });

    listen("call-ended", () => {
      this.showToast("Call ended");
      this.endCall();
    });

    listen<any>("voice-packet", (event) => {
      this.handleVoicePacket(event.payload);
    });

    listen<any>("video-frame", (event) => {
      this.handleVideoFrame(event.payload);
    });

    listen<any>("screen-frame", (event) => {
      this.handleScreenFrame(event.payload);
    });

    listen<any>("new-group-message", (event) => {
      const msg = event.payload;
      if (this.currentGroup && this.currentGroup.id === msg.group_id) {
        const msgs = store.getGroupMessagesForGroup(msg.group_id);
        msgs.push(msg);
        store.setGroupMessagesForGroup(msg.group_id, msgs);
        this.renderGroupMessages(msgs);
        this.scrollToBottom();
      }
    });

    listen<{peer_onion: string, is_typing: boolean}>("typing-indicator", (event) => {
      const payload = event.payload;
      if (this.currentContact && this.currentContact.onion_address === payload.peer_onion) {
        const indicator = document.getElementById("chat-typing-indicator");
        if (indicator) {
          if (payload.is_typing) {
            indicator.textContent = `${this.currentContact.display_name} is typing...`;
            indicator.classList.remove("hidden");
          } else {
            indicator.classList.add("hidden");
          }
        }
      }
    });
  }

  handleVoicePacket(payload: any): void {
    if (!payload || !payload.data || !Array.isArray(payload.data)) return;
    const bytes = new Uint8Array(payload.data);
    const blob = new Blob([bytes], { type: "audio/mp4" });
    this.voiceQ.push(blob);
    if (!this.voicePlaying) this.playVoiceQueue();
  }

  playVoiceQueue(): void {
    if (this.voiceQ.length === 0) {
      this.voicePlaying = false;
      return;
    }
    this.voicePlaying = true;
    const blob = this.voiceQ.shift()!;
    const audioUrl = URL.createObjectURL(blob);
    const audio = new Audio(audioUrl);
    audio.onended = () => {
      URL.revokeObjectURL(audioUrl);
      this.playVoiceQueue();
    };
    audio.play().catch(() => {
      this.playVoiceQueue();
    });
  }

  handleVideoFrame(payload: any): void {
    if (!payload || !payload.data) return;
    this.renderRemoteFrame(payload.data, "call-remote-container", false);
  }

  handleScreenFrame(payload: any): void {
    if (!payload || !payload.data) return;
    this.renderRemoteFrame(payload.data, "call-remote-container", true);
  }

  renderRemoteFrame(dataArr: number[], containerId: string, _isScreen: boolean): void {
    const container = document.getElementById(containerId);
    if (!container) return;
    const bytes = new Uint8Array(dataArr);
    let binary = "";
    for (const b of bytes) binary += String.fromCharCode(b);
    const dataUrl = "data:image/jpeg;base64," + btoa(binary);

    let img = container.querySelector("img") as HTMLImageElement | null;
    if (!img) {
      img = document.createElement("img");
      container.appendChild(img);
    }
    img.src = dataUrl;
  }

  setupSettings(): void {
    const toggles = [
      { id: "toggle-disappearing", key: "disappearing_messages_default" },
      { id: "toggle-read-receipts", key: "read_receipts" },
      { id: "toggle-typing-indicators", key: "typing_indicators" },
      { id: "toggle-notifications", key: "notifications_enabled" },
    ];

    for (const toggle of toggles) {
      const el = document.getElementById(toggle.id) as HTMLInputElement | null;
      if (el) {
        el.addEventListener("change", async () => {
          try {
            await api.updateSettings({ [toggle.key]: el.checked });
          } catch (err) {
            console.error("Failed to update setting:", err);
          }
        });
      }
    }

    const themeSelect = document.getElementById("settings-theme") as HTMLSelectElement | null;
    if (themeSelect) {
      themeSelect.addEventListener("change", async () => {
        try {
          await api.updateSettings({ theme: themeSelect.value });
        } catch (err) {
          console.error("Failed to update theme:", err);
        }
      });
    }
  }

  async updateTypingIndicators(): Promise<void> {
    if (!this.currentContact) return;
    try {
      const status: TypingStatus = await api.getTypingStatus(this.currentContact.onion_address);
      const indicator = document.getElementById("chat-typing-indicator");
      if (indicator) {
        if (status.is_typing) {
          indicator.textContent = `${this.currentContact.display_name} is typing...`;
          indicator.classList.remove("hidden");
        } else {
          indicator.classList.add("hidden");
        }
      }
    } catch {
      // silent
    }
  }

  async sendTyping(isTyping: boolean): Promise<void> {
    if (!this.currentContact) return;
    try {
      await api.sendTypingIndicator(this.currentContact.onion_address, isTyping);
      if (isTyping) {
        clearTimeout(this.typingTimeout);
        this.typingTimeout = setTimeout(() => this.sendTyping(false), 3000);
      }
    } catch {
      // silent
    }
  }

  showToast(msg: string): void {
    const toast = document.getElementById("toast");
    if (!toast) return;
    toast.textContent = msg;
    toast.classList.remove("hidden");
    setTimeout(() => toast.classList.add("hidden"), 3000);
  }

  avatarColor(name: string): string {
    let hash = 0;
    for (let i = 0; i < name.length; i++) {
      hash = (hash << 5) - hash + name.charCodeAt(i);
      hash |= 0;
    }
    const colors = ["avatar-red", "avatar-blue", "avatar-green", "avatar-purple", "avatar-orange", "avatar-pink", "avatar-teal", "avatar-indigo"];
    return colors[Math.abs(hash) % colors.length];
  }

  esc(text: string): string {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
  }

  formatTime(timestamp: number): string {
    const d = new Date(timestamp * 1000);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  formatDuration(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    return [h, m, s].map((v) => String(v).padStart(2, "0")).join(":");
  }

  formatDate(timestamp: number): string {
    const d = new Date(timestamp * 1000);
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 1);

    if (d >= today) return "Today";
    if (d >= yesterday) return "Yesterday";

    const diffMs = today.getTime() - d.getTime();
    const diffDays = Math.floor(diffMs / 86400000);
    if (diffDays < 7) {
      return d.toLocaleDateString([], { weekday: "long" });
    }
    return d.toLocaleDateString([], { month: "short", day: "numeric", year: d.getFullYear() !== now.getFullYear() ? "numeric" : undefined });
  }

  scrollToBottom(): void {
    const container = document.getElementById("chat-messages") || document.getElementById("group-chat-messages");
    if (container) {
      setTimeout(() => (container.scrollTop = container.scrollHeight), 50);
    }
  }

  private showModal(id: string): void {
    document.getElementById(id)?.classList.remove("hidden");
  }

  private hideModal(id: string): void {
    document.getElementById(id)?.classList.add("hidden");
  }

  private showContactInfo(contact: Contact): void {
    const nameEl = document.getElementById("contact-info-name");
    const onionEl = document.getElementById("contact-info-onion");
    if (nameEl) nameEl.textContent = contact.display_name;
    if (onionEl) onionEl.textContent = contact.onion_address;
    this.showModal("contact-info-modal");

    const verifyBtn = document.getElementById("contact-info-verify");
    const blockBtn = document.getElementById("contact-info-block");
    const deleteBtn = document.getElementById("contact-info-delete");

    const cleanup = () => {
      verifyBtn?.replaceWith(verifyBtn!.cloneNode(true));
      blockBtn?.replaceWith(blockBtn!.cloneNode(true));
      deleteBtn?.replaceWith(deleteBtn!.cloneNode(true));
    };
    cleanup();

    document.getElementById("contact-info-verify")?.addEventListener("click", async () => {
      try {
        await api.verifyContact(contact.onion_address);
        this.showToast("Contact verified");
        this.hideModal("contact-info-modal");
        await this.loadContacts();
      } catch { this.showToast("Failed to verify"); }
    });

    document.getElementById("contact-info-block")?.addEventListener("click", async () => {
      try {
        await api.blockContact(contact.onion_address);
        this.showToast("Contact blocked");
        this.hideModal("contact-info-modal");
        await this.loadContacts();
      } catch { this.showToast("Failed to block"); }
    });

    document.getElementById("contact-info-delete")?.addEventListener("click", async () => {
      try {
        await api.deleteContact(contact.onion_address);
        this.showToast("Contact deleted");
        this.hideModal("contact-info-modal");
        await this.loadContacts();
      } catch { this.showToast("Failed to delete"); }
    });
  }

  private async showGroupInfo(group: Group): Promise<void> {
    const nameEl = document.getElementById("group-info-name");
    const membersEl = document.getElementById("group-info-members");
    if (nameEl) nameEl.textContent = group.name;

    try {
      const members = await api.getGroupMembers(group.id);
      if (membersEl) {
        membersEl.innerHTML = members.map((m: GroupMember) => `
          <div class="group-member">
            <div class="avatar avatar-small ${this.avatarColor(m.display_name || m.onion_address)}">${(m.display_name || '?').charAt(0).toUpperCase()}</div>
            <span class="member-name">${this.esc(m.display_name || m.onion_address.slice(0, 16))}</span>
            <span class="member-role">${m.role}</span>
          </div>
        `).join("");
      }
    } catch (err) {
      console.error("Failed to load group members:", err);
      if (membersEl) {
        membersEl.innerHTML = `<div class="group-member"><span>${this.esc(group.created_by)}</span><span class="member-role">admin</span></div>`;
      }
    }

    this.showModal("group-info-modal");
  }

  private showReplyPreview(msg: Message): void {
    const container = document.getElementById("reply-preview-container");
    const textEl = document.getElementById("reply-preview-text");
    if (container && textEl) {
      textEl.textContent = msg.content || "Attachment";
      container.classList.remove("hidden");
    }
  }

  private clearReplyPreview(): void {
    document.getElementById("reply-preview-container")?.classList.add("hidden");
  }

  private setupReplyPreview(): void {
    document.getElementById("reply-cancel")?.addEventListener("click", () => {
      this.replyToMessage = null;
      this.clearReplyPreview();
    });
  }

  private async markAsRead(onion: string): Promise<void> {
    try {
      await api.markMessagesRead(onion);
      store.markContactMessagesRead(onion);
    } catch {
      // silent
    }
  }
}

const app = new VchatApp();
app.init();
