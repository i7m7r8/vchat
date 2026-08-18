import { api } from "./lib/api";
import { store } from "./lib/store";

class VchatApp {
  private currentContact: any = null;
  private activeCallId: string | null = null;
  private callTimerInterval: any = null;
  private callSeconds = 0;

  async init() {
    await this.loadIdentity();
    await this.loadContacts();
    this.setupNavigation();
    this.setupChatView();
    this.setupModals();
    this.setupCallOverlay();
    this.showScreen("chats");
  }

  private async loadIdentity() {
    try {
      let identity = await api.getIdentity();
      if (!identity) {
        identity = await api.initIdentity("User");
      }
      store.setIdentity(identity);
      this.updateSettingsProfile(identity);
    } catch (e) {
      console.error("Failed to load identity:", e);
    }
  }

  private updateSettingsProfile(identity: any) {
    const nameEl = document.getElementById("settings-name");
    const onionEl = document.getElementById("settings-onion");
    const avatarEl = document.getElementById("settings-avatar");
    if (nameEl) nameEl.textContent = identity.display_name;
    if (onionEl) onionEl.textContent = identity.onion_address;
    if (avatarEl) {
      avatarEl.className = `avatar large ${this.avatarColor(identity.display_name)}`;
      avatarEl.textContent = identity.display_name.charAt(0).toUpperCase();
    }
  }

  private async loadContacts() {
    try {
      const contacts = await api.getContacts();
      store.setContacts(contacts);
      this.renderContacts(contacts);
      this.renderChatList(contacts);
    } catch (e) {
      console.error("Failed to load contacts:", e);
    }
  }

  private setupNavigation() {
    document.querySelectorAll(".nav-item").forEach((btn) => {
      btn.addEventListener("click", () => {
        const screen = (btn as HTMLElement).dataset.screen;
        if (screen) this.showScreen(screen);
      });
    });

    document.getElementById("fab-new-chat")?.addEventListener("click", () => {
      this.showScreen("contacts");
    });
  }

  private showScreen(name: string) {
    const screens = ["chats", "chat", "contacts", "calls", "settings"];
    screens.forEach((s) => {
      const el = document.getElementById(`screen-${s}`);
      if (el) el.style.display = s === name ? "flex" : "none";
    });

    const bottomNav = document.getElementById("bottom-nav");
    if (bottomNav) {
      bottomNav.style.display = ["chats", "contacts", "calls", "settings"].includes(name) ? "flex" : "none";
    }

    document.querySelectorAll(".nav-item").forEach((btn) => {
      const el = btn as HTMLElement;
      el.classList.toggle("active", el.dataset.screen === name);
    });
  }

  private renderChatList(contacts: any[]) {
    const list = document.getElementById("chat-list");
    if (!list) return;
    list.innerHTML = "";

    if (contacts.length === 0) {
      list.innerHTML = `<div class="empty-screen"><div class="empty-icon"><svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg></div><h3>No conversations yet</h3><p>Start chatting by adding a contact</p></div>`;
      return;
    }

    contacts.forEach((contact) => {
      const msgs = store.getMessages(contact.onion_address);
      const lastMsg = msgs.length > 0 ? msgs[msgs.length - 1] : null;
      const timeStr = lastMsg
        ? new Date(lastMsg.timestamp * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : "";
      const preview = lastMsg ? lastMsg.content : "Tap to start chatting";

      const item = document.createElement("div");
      item.className = "chat-item";
      item.innerHTML = `
        <div class="avatar ${this.avatarColor(contact.display_name)}">
          ${contact.display_name.charAt(0).toUpperCase()}
          <div class="online-dot"></div>
        </div>
        <div class="chat-item-info">
          <div class="chat-item-row">
            <div class="chat-item-name">${this.esc(contact.display_name)}</div>
            <div class="chat-item-time">${timeStr}</div>
          </div>
          <div class="chat-item-preview">${this.esc(preview)}</div>
        </div>
      `;
      item.addEventListener("click", () => this.openChat(contact));
      list.appendChild(item);
    });
  }

  private renderContacts(contacts: any[]) {
    const list = document.getElementById("contacts-list");
    if (!list) return;
    list.innerHTML = "";

    if (contacts.length === 0) {
      list.innerHTML = `<div class="empty-screen"><div class="empty-icon"><svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg></div><h3>No contacts yet</h3><p>Add contacts to start chatting</p></div>`;
      return;
    }

    contacts.forEach((contact) => {
      const item = document.createElement("div");
      item.className = "contact-item";
      const shortKey = contact.onion_address.substring(0, 20) + "...";
      item.innerHTML = `
        <div class="avatar ${this.avatarColor(contact.display_name)}">
          ${contact.display_name.charAt(0).toUpperCase()}
        </div>
        <div class="contact-item-info">
          <div class="contact-item-name">${this.esc(contact.display_name)}</div>
          <div class="contact-item-sub">${this.esc(shortKey)}</div>
        </div>
      `;
      item.addEventListener("click", () => this.openChat(contact));
      list.appendChild(item);
    });
  }

  private openChat(contact: any) {
    this.currentContact = contact;
    store.setSelectedContact(contact);

    document.getElementById("chat-view-avatar")!.className = `avatar ${this.avatarColor(contact.display_name)}`;
    document.getElementById("chat-view-avatar")!.textContent = contact.display_name.charAt(0).toUpperCase();
    document.getElementById("chat-view-name")!.textContent = contact.display_name;
    document.getElementById("chat-view-status")!.textContent = "Online";

    this.showScreen("chat");
    this.loadMessages(contact.onion_address);
  }

  private setupChatView() {
    document.getElementById("btn-back-chat")?.addEventListener("click", () => {
      this.showScreen("chats");
      this.renderChatList(store.getContacts());
    });

    document.getElementById("btn-call")?.addEventListener("click", () => {
      this.startCall(false);
    });

    document.getElementById("btn-video")?.addEventListener("click", () => {
      this.startCall(true);
    });

    document.getElementById("btn-send")?.addEventListener("click", () => {
      this.sendMessage();
    });

    document.getElementById("message-input")?.addEventListener("keydown", (e: KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        this.sendMessage();
      }
    });

    const textarea = document.getElementById("message-input") as HTMLTextAreaElement;
    if (textarea) {
      textarea.addEventListener("input", () => {
        textarea.style.height = "auto";
        textarea.style.height = Math.min(textarea.scrollHeight, 120) + "px";
      });
    }

    document.getElementById("btn-search-chats")?.addEventListener("click", () => {
      const bar = document.getElementById("search-chats-bar");
      if (bar) bar.style.display = bar.style.display === "none" ? "flex" : "none";
    });
  }

  private async loadMessages(contactOnion: string) {
    try {
      const messages = await api.getMessages(contactOnion);
      store.setMessages(contactOnion, messages);
      this.renderMessages(messages);
    } catch (e) {
      console.error("Failed to load messages:", e);
    }
  }

  private renderMessages(messages: any[]) {
    const list = document.getElementById("messages-list");
    if (!list) return;
    list.innerHTML = "";

    if (messages.length === 0) {
      list.innerHTML = `<div class="empty-screen" style="padding:40px"><p style="color:var(--on-surface-dim)">Messages are end-to-end encrypted. Say hello!</p></div>`;
      return;
    }

    const identity = store.getIdentity();
    const myAddr = identity?.onion_address;

    messages.forEach((msg) => {
      const isSent = msg.sender === myAddr;
      const time = new Date(msg.timestamp * 1000).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });

      const el = document.createElement("div");
      el.className = `message-bubble ${isSent ? "sent" : "received"}`;
      el.innerHTML = `
        <div class="msg-text">${this.esc(msg.content)}</div>
        <div class="msg-meta">
          <span class="msg-lock">&#128274;</span>
          <span class="msg-time">${time}</span>
        </div>
      `;
      list.appendChild(el);
    });

    const area = document.getElementById("messages-area");
    if (area) area.scrollTop = area.scrollHeight;
  }

  private async sendMessage() {
    const input = document.getElementById("message-input") as HTMLTextAreaElement;
    if (!input || !input.value.trim() || !this.currentContact) return;

    const content = input.value.trim();
    input.value = "";
    input.style.height = "auto";

    try {
      const msg = await api.sendMessage(this.currentContact.onion_address, content, "Text");
      store.addMessage(this.currentContact.onion_address, msg);
      this.renderMessages(store.getMessages(this.currentContact.onion_address));
    } catch (e) {
      console.error("Failed to send:", e);
      this.showToast("Failed to send message");
    }
  }

  private setupModals() {
    document.getElementById("fab-add-contact")?.addEventListener("click", () => {
      (document.getElementById("modal-add-contact") as HTMLElement).style.display = "flex";
    });

    const closeModal = (id: string) => {
      const el = document.getElementById(id) as HTMLElement;
      if (el) el.style.display = "none";
    };

    document.getElementById("btn-close-add-contact")?.addEventListener("click", () => closeModal("modal-add-contact"));
    document.getElementById("btn-cancel-add-contact")?.addEventListener("click", () => closeModal("modal-add-contact"));

    document.getElementById("btn-save-add-contact")?.addEventListener("click", async () => {
      const name = (document.getElementById("input-contact-name") as HTMLInputElement).value;
      const onion = (document.getElementById("input-contact-onion") as HTMLInputElement).value;
      const key = (document.getElementById("input-contact-key") as HTMLInputElement).value;

      if (!name || !onion || !key) {
        this.showToast("Please fill in all fields");
        return;
      }

      try {
        const contact = await api.addContact(name, key, onion);
        store.addContact(contact);
        this.renderContacts(store.getContacts());
        this.renderChatList(store.getContacts());
        closeModal("modal-add-contact");
        (document.getElementById("input-contact-name") as HTMLInputElement).value = "";
        (document.getElementById("input-contact-onion") as HTMLInputElement).value = "";
        (document.getElementById("input-contact-key") as HTMLInputElement).value = "";
        this.showToast("Contact added");
      } catch (e) {
        this.showToast("Failed to add contact");
      }
    });

    document.getElementById("btn-show-qr")?.addEventListener("click", async () => {
      try {
        const qrData = await api.generateQrCode();
        const container = document.getElementById("qr-display");
        if (container) {
          container.innerHTML = `<img src="${qrData}" alt="QR Code" style="width:180px;height:180px;border-radius:8px"/>`;
        }
        (document.getElementById("modal-qr") as HTMLElement).style.display = "flex";
      } catch (e) {
        this.showToast("Failed to generate QR code");
      }
    });

    document.getElementById("btn-close-qr")?.addEventListener("click", () => closeModal("modal-qr"));
    document.getElementById("btn-edit-name")?.addEventListener("click", () => {
      const identity = store.getIdentity();
      if (identity) {
        (document.getElementById("input-new-name") as HTMLInputElement).value = identity.display_name;
      }
      (document.getElementById("modal-edit-name") as HTMLElement).style.display = "flex";
    });

    document.getElementById("btn-close-edit-name")?.addEventListener("click", () => closeModal("modal-edit-name"));
    document.getElementById("btn-cancel-edit-name")?.addEventListener("click", () => closeModal("modal-edit-name"));

    document.getElementById("btn-save-edit-name")?.addEventListener("click", async () => {
      const newName = (document.getElementById("input-new-name") as HTMLInputElement).value.trim();
      if (!newName) return;
      try {
        const identity = await api.initIdentity(newName);
        store.setIdentity(identity);
        this.updateSettingsProfile(identity);
        closeModal("modal-edit-name");
        this.showToast("Name updated");
      } catch (e) {
        this.showToast("Failed to update name");
      }
    });
  }

  private setupCallOverlay() {
    document.getElementById("btn-end-call")?.addEventListener("click", () => this.endCall());

    document.getElementById("btn-mute")?.addEventListener("click", (e) => {
      (e.currentTarget as HTMLElement).classList.toggle("active");
    });

    document.getElementById("btn-speaker")?.addEventListener("click", (e) => {
      (e.currentTarget as HTMLElement).classList.toggle("active");
    });

    document.getElementById("btn-video-toggle")?.addEventListener("click", (e) => {
      (e.currentTarget as HTMLElement).classList.toggle("active");
    });

    document.getElementById("btn-screen-toggle")?.addEventListener("click", (e) => {
      (e.currentTarget as HTMLElement).classList.toggle("active");
    });
  }

  private async startCall(_video: boolean) {
    if (!this.currentContact) return;

    try {
      const callId = await api.startVideoCall(this.currentContact.onion_address);
      this.activeCallId = callId;

      document.getElementById("call-avatar")!.className = `avatar large ${this.avatarColor(this.currentContact.display_name)}`;
      document.getElementById("call-avatar")!.textContent = this.currentContact.display_name.charAt(0).toUpperCase();
      document.getElementById("call-name")!.textContent = this.currentContact.display_name;
      document.getElementById("call-status")!.textContent = "Calling...";

      document.getElementById("call-overlay")!.style.display = "flex";

      setTimeout(() => {
        const statusEl = document.getElementById("call-status");
        if (statusEl && this.activeCallId) {
          statusEl.textContent = "Connected";
          document.getElementById("call-timer")!.style.display = "block";
          this.callSeconds = 0;
          this.callTimerInterval = setInterval(() => {
            this.callSeconds++;
            const m = Math.floor(this.callSeconds / 60).toString().padStart(2, "0");
            const s = (this.callSeconds % 60).toString().padStart(2, "0");
            document.getElementById("call-timer")!.textContent = `${m}:${s}`;
          }, 1000);
        }
      }, 2000);
    } catch (e) {
      this.showToast("Failed to start call");
    }
  }

  private async endCall() {
    if (this.activeCallId) {
      try {
        await api.endVideoCall(this.activeCallId);
      } catch (e) {
        console.error("Failed to end call:", e);
      }
    }

    this.activeCallId = null;
    if (this.callTimerInterval) {
      clearInterval(this.callTimerInterval);
      this.callTimerInterval = null;
    }
    document.getElementById("call-timer")!.style.display = "none";
    document.getElementById("call-overlay")!.style.display = "none";

    document.querySelectorAll(".call-control-btn").forEach((btn) => {
      btn.classList.remove("active");
    });
  }

  private showToast(msg: string) {
    const toast = document.getElementById("toast");
    if (toast) {
      toast.textContent = msg;
      toast.classList.add("show");
      setTimeout(() => toast.classList.remove("show"), 2500);
    }
  }

  private avatarColor(name: string): string {
    let hash = 0;
    for (let i = 0; i < name.length; i++) {
      hash = name.charCodeAt(i) + ((hash << 5) - hash);
    }
    return `av-${Math.abs(hash) % 8}`;
  }

  private esc(text: string): string {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
  }
}

const app = new VchatApp();
app.init();
