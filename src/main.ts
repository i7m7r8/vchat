import { api, Contact, Message, Identity, Group, GroupMessage, Reaction, TypingStatus, CallLogEntry, AppSettings } from "./lib/api";
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

  async init(): Promise<void> {
    try {
      await this.initDatabase();
      await this.loadIdentity();
      await this.loadContacts();
      await this.loadGroups();
      await this.updateTorStatus();
      this.setupNavigation();
      this.setupChatView();
      this.setupGroupChatView();
      this.setupModals();
      this.setupCallOverlay();
      this.setupContextMenu();
      this.setupEmojiPicker();
      this.setupSearch();
      this.showScreen("chats");

      setInterval(() => this.updateTorStatus(), 30000);
      setInterval(() => this.updateTypingIndicators(), 5000);
    } catch (err) {
      console.error("Init failed:", err);
      this.showToast("Failed to initialize app");
    }
  }

  async initDatabase(): Promise<void> {
    await api.getContacts();
  }

  async loadIdentity(): Promise<void> {
    try {
      let identity: Identity | null = await api.getIdentity();
      if (!identity) {
        identity = await api.createIdentity();
      }
      store.identity = identity;
      this.updateSettingsProfile(identity);
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
        badge.className = `tor-badge tor-${status}`;
        badge.textContent = status === "connected" ? "Tor Connected" : status === "connecting" ? "Connecting..." : "Tor Offline";
      }
    } catch (err) {
      console.error("Tor status check failed:", err);
    }
  }

  async loadContacts(): Promise<void> {
    try {
      const contacts = await api.getContacts();
      store.contacts = contacts;
      this.renderChatList(contacts);
      this.renderContacts(contacts);
    } catch (err) {
      console.error("Failed to load contacts:", err);
    }
  }

  async loadGroups(): Promise<void> {
    try {
      const groups = await api.getGroups();
      store.groups = groups;
      this.renderGroupsList(groups);
    } catch (err) {
      console.error("Failed to load groups:", err);
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
        const lastMsg = store.lastMessages[c.onion_address];
        const preview = lastMsg ? this.esc(lastMsg.text || "Attachment") : "No messages yet";
        const time = lastMsg ? this.formatDate(lastMsg.timestamp) : "";
        const unread = store.unreadCounts[c.onion_address] || 0;
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
          <span class="group-list-members">${g.members.length} member${g.members.length !== 1 ? "s" : ""}</span>
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
        const dirIcon = call.direction === "outgoing" ? "↗" : call.direction === "missed" ? "✕" : "↙";
        const typeIcon = call.call_type === "video" ? "📹" : "📞";
        return `
        <div class="call-history-item" data-call-id="${this.esc(call.id)}">
          <div class="call-icon ${call.direction === "missed" ? "missed" : ""}">${dirIcon}</div>
          <div class="call-type-icon">${typeIcon}</div>
          <div class="call-info">
            <span class="call-name">${this.esc(call.contact_name || call.contact_onion)}</span>
            <span class="call-date">${this.formatDate(call.timestamp)}</span>
          </div>
          <span class="call-duration">${call.duration ? this.formatDuration(call.duration) : ""}</span>
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
          (item as HTMLElement).style.display = call.direction === "missed" ? "" : "none";
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
    if (headerInfo) headerInfo.textContent = `${group.members.length} members`;

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

    document.getElementById("chat-voice-btn")?.addEventListener("click", () => {
      this.showToast("Voice notes coming soon");
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
      store.messages[contactOnion] = messages;
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
      store.groupMessages[groupId] = messages;
      this.renderGroupMessages(messages);
    } catch (err) {
      console.error("Failed to load group messages:", err);
      this.showToast("Failed to load messages");
    }
  }

  renderMessages(messages: Message[]): void {
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

    let html = "";
    let lastDate = "";

    for (const msg of messages) {
      const dateStr = this.formatDate(msg.timestamp);
      if (dateStr !== lastDate) {
        html += `<div class="date-separator"><span>${dateStr}</span></div>`;
        lastDate = dateStr;
      }

      const isSent = msg.direction === "sent";
      const statusIcon = isSent
        ? msg.status === "read"
          ? '<span class="msg-status msg-read">✓✓</span>'
          : msg.status === "delivered"
          ? '<span class="msg-status msg-delivered">✓✓</span>'
          : '<span class="msg-status msg-sent">✓</span>'
        : "";

      const lockIcon = '<span class="msg-lock">🔒</span>';
      const expiryIcon = msg.expires_at ? '<span class="msg-expiry">⏱</span>' : "";

      let replyPreview = "";
      if (msg.reply_to) {
        replyPreview = `
          <div class="reply-preview">
            <div class="reply-preview-text">${this.esc(msg.reply_to.text || "Attachment")}</div>
          </div>`;
      }

      let reactionsHtml = "";
      if (msg.reactions && msg.reactions.length > 0) {
        const reactionMap = new Map<string, number>();
        msg.reactions.forEach((r: Reaction) => {
          reactionMap.set(r.emoji, (reactionMap.get(r.emoji) || 0) + 1);
        });
        const items = Array.from(reactionMap.entries())
          .map(([emoji, count]) => `<span class="reaction-chip">${emoji} ${count > 1 ? count : ""}</span>`)
          .join("");
        reactionsHtml = `<div class="message-reactions">${items}</div>`;
      }

      const bodyHtml = msg.text ? `<div class="msg-text">${this.esc(msg.text)}</div>` : "";
      const attachmentHtml = msg.file_path
        ? `<div class="msg-attachment">📎 ${this.esc(msg.file_name || "File")}</div>`
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

    let html = "";
    let lastDate = "";

    for (const msg of messages) {
      const dateStr = this.formatDate(msg.timestamp);
      if (dateStr !== lastDate) {
        html += `<div class="date-separator"><span>${dateStr}</span></div>`;
        lastDate = dateStr;
      }

      const isSent = msg.sender_onion === store.identity?.onion_address;
      const senderName = isSent ? "You" : this.esc(msg.sender_name || msg.sender_onion.slice(0, 12));

      let replyPreview = "";
      if (msg.reply_to) {
        replyPreview = `
          <div class="reply-preview">
            <div class="reply-preview-text">${this.esc(msg.reply_to.text || "Attachment")}</div>
          </div>`;
      }

      const bodyHtml = msg.text ? `<div class="msg-text">${this.esc(msg.text)}</div>` : "";
      const attachmentHtml = msg.file_path
        ? `<div class="msg-attachment">📎 ${this.esc(msg.file_name || "File")}</div>`
        : "";

      html += `
        <div class="message ${isSent ? "sent" : "received"}" data-msg-id="${this.esc(msg.id)}">
          ${!isSent ? `<div class="msg-sender">${senderName}</div>` : ""}
          ${replyPreview}
          <div class="msg-bubble">
            ${bodyHtml}
            ${attachmentHtml}
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
      const msg = await api.sendMessage(this.currentContact.onion_address, text || "", this.replyToMessage?.id);
      if (!store.messages[this.currentContact.onion_address]) {
        store.messages[this.currentContact.onion_address] = [];
      }
      store.messages[this.currentContact.onion_address].push(msg);
      store.lastMessages[this.currentContact.onion_address] = msg;
      this.renderMessages(store.messages[this.currentContact.onion_address]);
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
      const msg = await api.sendGroupMessage(this.currentGroup.id, text);
      if (!store.groupMessages[this.currentGroup.id]) {
        store.groupMessages[this.currentGroup.id] = [];
      }
      store.groupMessages[this.currentGroup.id].push(msg);
      this.renderGroupMessages(store.groupMessages[this.currentGroup.id]);
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
      const name = nameInput?.value.trim();
      const onion = onionInput?.value.trim();
      if (!name || !onion) {
        this.showToast("Name and onion address required");
        return;
      }
      try {
        await api.addContact(name, onion);
        await this.loadContacts();
        this.hideModal("add-contact-modal");
        if (nameInput) nameInput.value = "";
        if (onionInput) onionInput.value = "";
        this.showToast("Contact added");
      } catch (err) {
        console.error("Failed to add contact:", err);
        this.showToast("Failed to add contact");
      }
    });

    document.getElementById("qr-code-open")?.addEventListener("click", async () => {
      this.showModal("qr-code-modal");
      const qrImg = document.getElementById("qr-code-image") as HTMLImageElement | null;
      if (qrImg && store.identity) {
        try {
          const dataUrl = await api.generateQR(store.identity.onion_address);
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
        const result = await api.scanQRFromCamera();
        if (result) {
          this.hideModal("qr-code-modal");
          const onionInput = document.getElementById("add-contact-onion") as HTMLInputElement | null;
          if (onionInput) onionInput.value = result;
          this.showModal("add-contact-modal");
        }
      } catch (err) {
        this.showToast("QR scan failed");
      }
    });

    document.getElementById("qr-scan-file")?.addEventListener("click", async () => {
      try {
        const result = await api.scanQRFromFile();
        if (result) {
          this.hideModal("qr-code-modal");
          const onionInput = document.getElementById("add-contact-onion") as HTMLInputElement | null;
          if (onionInput) onionInput.value = result;
          this.showModal("add-contact-modal");
        }
      } catch (err) {
        this.showToast("QR scan failed");
      }
    });

    document.getElementById("edit-name-open")?.addEventListener("click", () => {
      const input = document.getElementById("edit-name-input") as HTMLInputElement | null;
      if (input && store.identity) input.value = store.identity.display_name;
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
        await api.updateIdentity(name);
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
        await api.createGroup(name, []);
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
        await api.addGroupMember(this.currentGroup.id, onion);
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
            await this.handleFileSend((reader.result as string).split(",")[1]);
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
      api.toggleMute();
    });

    document.getElementById("call-speaker-btn")?.addEventListener("click", () => {
      const btn = document.getElementById("call-speaker-btn");
      btn?.classList.toggle("active");
      api.toggleSpeaker();
    });

    document.getElementById("call-end-btn")?.addEventListener("click", () => {
      this.endCall();
    });

    document.getElementById("call-video-btn")?.addEventListener("click", () => {
      api.toggleVideo();
      const btn = document.getElementById("call-video-btn");
      btn?.classList.toggle("active");
    });

    document.getElementById("call-screen-btn")?.addEventListener("click", () => {
      api.toggleScreenShare();
      const btn = document.getElementById("call-screen-btn");
      btn?.classList.toggle("active");
    });
  }

  async startCall(video: boolean): Promise<void> {
    if (!this.currentContact) return;
    try {
      const callId = await api.startCall(this.currentContact.onion_address, video);
      this.activeCallId = callId;
      this.callSeconds = 0;

      const overlay = document.getElementById("call-overlay");
      overlay?.classList.remove("hidden");

      const statusEl = document.getElementById("call-status");
      if (statusEl) statusEl.textContent = "Ringing...";

      const nameEl = document.getElementById("call-name");
      if (nameEl) nameEl.textContent = this.currentContact.display_name;

      this.callTimerInterval = setInterval(() => {
        this.callSeconds++;
        const timerEl = document.getElementById("call-timer");
        if (timerEl) timerEl.textContent = this.formatDuration(this.callSeconds);
      }, 1000);
    } catch (err) {
      console.error("Failed to start call:", err);
      this.showToast("Failed to start call");
    }
  }

  async endCall(): Promise<void> {
    if (this.activeCallId) {
      try {
        await api.endCall(this.activeCallId);
      } catch (err) {
        console.error("Failed to end call:", err);
      }
    }

    this.activeCallId = null;
    clearInterval(this.callTimerInterval);
    this.callTimerInterval = null;
    this.callSeconds = 0;

    const overlay = document.getElementById("call-overlay");
    overlay?.classList.add("hidden");
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
      if (this.contextMenuTarget?.text) {
        try {
          await navigator.clipboard.writeText(this.contextMenuTarget.text);
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
          const msgs = store.messages[this.currentContact.onion_address] || [];
          store.messages[this.currentContact.onion_address] = msgs.filter((m) => m.id !== this.contextMenuTarget!.id);
          this.renderMessages(store.messages[this.currentContact.onion_address]);
          this.showToast("Message deleted");
        } catch (err) {
          this.showToast("Failed to delete message");
        }
      }
      this.hideContextMenu();
    });

    document.getElementById("ctx-forward")?.addEventListener("click", () => {
      this.showToast("Forward coming soon");
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
      "☀️","🌙","⭐","🌸","🍕","🍔","☕","🎂","🎮","⚽",
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

  async handleFileSend(filePath: string): Promise<void> {
    if (!this.currentContact && !this.currentGroup) return;
    try {
      const progressEl = document.getElementById("file-send-progress");
      if (progressEl) progressEl.classList.remove("hidden");

      if (this.currentContact) {
        await api.sendFile(this.currentContact.onion_address, filePath);
      } else if (this.currentGroup) {
        await api.sendGroupFile(this.currentGroup.id, filePath);
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
      const filtered = store.contacts.filter(
        (c) => c.display_name.toLowerCase().includes(query) || c.onion_address.toLowerCase().includes(query)
      );
      this.renderChatList(filtered);
    });

    const contactSearch = document.getElementById("contacts-search-input") as HTMLInputElement | null;
    contactSearch?.addEventListener("input", () => {
      const query = contactSearch.value.toLowerCase();
      const filtered = store.contacts.filter(
        (c) => c.display_name.toLowerCase().includes(query) || c.onion_address.toLowerCase().includes(query)
      );
      this.renderContacts(filtered);
    });
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
    } catch (err) {
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
    } catch (err) {
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
  }

  private showGroupInfo(group: Group): void {
    const nameEl = document.getElementById("group-info-name");
    const membersEl = document.getElementById("group-info-members");
    if (nameEl) nameEl.textContent = group.name;
    if (membersEl) {
      membersEl.innerHTML = group.members
        .map((m: any) => `<div class="group-member"><span>${this.esc(m.name || m.onion_address)}</span></div>`)
        .join("");
    }
    this.showModal("group-info-modal");
  }

  private showReplyPreview(msg: Message): void {
    const container = document.getElementById("reply-preview-container");
    const textEl = document.getElementById("reply-preview-text");
    if (container && textEl) {
      textEl.textContent = msg.text || "Attachment";
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
      await api.markAsRead(onion);
      store.unreadCounts[onion] = 0;
    } catch (err) {
      // silent
    }
  }
}

const app = new VchatApp();
app.init();
