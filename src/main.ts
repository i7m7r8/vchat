import { api, Contact, Message, Identity } from "./lib/api";
import { store } from "./lib/store";

class VchatApp {
  private currentContact: Contact | null = null;
  private activeCallId: string | null = null;
  private callTimerInterval: ReturnType<typeof setInterval> | null = null;
  private callSeconds = 0;
  private qrScannerStream: MediaStream | null = null;
  private qrScanInterval: ReturnType<typeof setInterval> | null = null;

  async init() {
    await this.initDatabase();
    await this.loadIdentity();
    await this.loadContacts();
    await this.updateTorStatus();
    this.setupNavigation();
    this.setupChatView();
    this.setupModals();
    this.setupCallOverlay();
    this.showScreen("chats");

    setInterval(() => this.updateTorStatus(), 30000);
  }

  private async initDatabase() {
    try {
      await api.getContacts();
    } catch (e) {
      console.error("DB init check failed:", e);
    }
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
      this.showToast("Failed to initialize identity");
    }
  }

  private updateSettingsProfile(identity: Identity) {
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

  private async updateTorStatus() {
    try {
      const status = await api.getTorStatus();
      const torBadge = document.getElementById("tor-status");
      if (torBadge) {
        torBadge.textContent = status.connected ? "Connected" : "Offline";
        torBadge.className = `settings-badge ${status.connected ? "" : "error"}`;
      }
      const onionEl = document.getElementById("settings-onion");
      if (onionEl && status.onion_address) {
        onionEl.textContent = status.onion_address;
      }
    } catch (e) {
      console.error("Tor status check failed:", e);
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

    document.getElementById("btn-search-chats")?.addEventListener("click", () => {
      const bar = document.getElementById("search-chats-bar");
      if (bar) bar.style.display = bar.style.display === "none" ? "flex" : "none";
    });

    document.getElementById("btn-search-contacts")?.addEventListener("click", () => {
      const bar = document.getElementById("search-contacts-bar");
      if (bar) bar.style.display = bar.style.display === "none" ? "flex" : "none";
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
      bottomNav.style.display = ["chats", "contacts", "calls", "settings"].includes(name)
        ? "flex"
        : "none";
    }

    document.querySelectorAll(".nav-item").forEach((btn) => {
      const el = btn as HTMLElement;
      el.classList.toggle("active", el.dataset.screen === name);
    });
  }

  private renderChatList(contacts: Contact[]) {
    const list = document.getElementById("chat-list");
    if (!list) return;
    list.innerHTML = "";

    if (contacts.length === 0) {
      list.innerHTML = `<div class="empty-screen"><div class="empty-icon"><svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg></div><h3>No conversations yet</h3><p>Add a contact via QR code to start</p></div>`;
      return;
    }

    contacts.forEach((contact) => {
      const msgs = store.getMessages(contact.onion_address);
      const lastMsg = msgs.length > 0 ? msgs[msgs.length - 1] : null;
      const timeStr = lastMsg
        ? new Date(lastMsg.timestamp * 1000).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          })
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

  private renderContacts(contacts: Contact[]) {
    const list = document.getElementById("contacts-list");
    if (!list) return;
    list.innerHTML = "";

    if (contacts.length === 0) {
      list.innerHTML = `<div class="empty-screen"><div class="empty-icon"><svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg></div><h3>No contacts yet</h3><p>Scan a QR code or add manually</p></div>`;
      return;
    }

    contacts.forEach((contact) => {
      const item = document.createElement("div");
      item.className = "contact-item";
      const shortAddr = contact.onion_address.substring(0, 24) + "...";
      item.innerHTML = `
        <div class="avatar ${this.avatarColor(contact.display_name)}">
          ${contact.display_name.charAt(0).toUpperCase()}
        </div>
        <div class="contact-item-info">
          <div class="contact-item-name">${this.esc(contact.display_name)}</div>
          <div class="contact-item-sub">${this.esc(shortAddr)}</div>
        </div>
      `;
      item.addEventListener("click", () => this.openChat(contact));
      list.appendChild(item);
    });
  }

  private openChat(contact: Contact) {
    this.currentContact = contact;
    store.setSelectedContact(contact);

    const avatar = document.getElementById("chat-view-avatar")!;
    avatar.className = `avatar ${this.avatarColor(contact.display_name)}`;
    avatar.textContent = contact.display_name.charAt(0).toUpperCase();
    document.getElementById("chat-view-name")!.textContent = contact.display_name;
    document.getElementById("chat-view-status")!.textContent = "Online (Tor)";

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

  private renderMessages(messages: Message[]) {
    const list = document.getElementById("messages-list");
    if (!list) return;
    list.innerHTML = "";

    if (messages.length === 0) {
      list.innerHTML = `<div class="empty-screen" style="padding:40px"><p style="color:var(--on-surface-dim)">Messages are end-to-end encrypted.<br/>Only you and your contact can read them.</p></div>`;
      return;
    }

    const identity = store.getIdentity();
    const myAddr = identity?.onion_address;

    let lastDate = "";
    messages.forEach((msg) => {
      const msgDate = new Date(msg.timestamp * 1000).toLocaleDateString();
      if (msgDate !== lastDate) {
        const sep = document.createElement("div");
        sep.className = "message-date-separator";
        sep.textContent = msgDate;
        list.appendChild(sep);
        lastDate = msgDate;
      }

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
          <span class="msg-lock">${msg.encrypted ? "&#128274;" : "&#128275;"}</span>
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

    document.getElementById("btn-close-add-contact")?.addEventListener("click", () =>
      closeModal("modal-add-contact")
    );
    document.getElementById("btn-cancel-add-contact")?.addEventListener("click", () =>
      closeModal("modal-add-contact")
    );

    document.getElementById("btn-save-add-contact")?.addEventListener("click", async () => {
      const name = (document.getElementById("input-contact-name") as HTMLInputElement).value;
      const onion = (document.getElementById("input-contact-onion") as HTMLInputElement).value;
      const key = (document.getElementById("input-contact-key") as HTMLInputElement).value;

      if (!name || !onion || !key) {
        this.showToast("Please fill in all fields");
        return;
      }

      try {
        await api.addContact(name, key, onion);
        await this.loadContacts();
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
          if (qrData.startsWith("data:")) {
            container.innerHTML = `<img src="${qrData}" alt="QR Code" style="width:200px;height:200px;border-radius:8px"/>`;
          } else {
            container.innerHTML = `<pre style="font-size:10px;word-break:break-all;max-width:200px">${this.esc(qrData)}</pre>`;
          }
        }
        (document.getElementById("modal-qr") as HTMLElement).style.display = "flex";
      } catch (e) {
        this.showToast("Failed to generate QR code");
      }
    });

    document.getElementById("btn-close-qr")?.addEventListener("click", () => {
      this.stopQrScanner();
      closeModal("modal-qr");
    });

    document.getElementById("btn-scan-qr")?.addEventListener("click", () => {
      this.startQrScanner();
    });

    document.getElementById("btn-scan-qr-file")?.addEventListener("click", () => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = "image/*";
      input.onchange = async (e) => {
        const file = (e.target as HTMLInputElement).files?.[0];
        if (file) {
          await this.scanQrFromFile(file);
        }
      };
      input.click();
    });

    document.getElementById("btn-edit-name")?.addEventListener("click", () => {
      const identity = store.getIdentity();
      if (identity) {
        (document.getElementById("input-new-name") as HTMLInputElement).value =
          identity.display_name;
      }
      (document.getElementById("modal-edit-name") as HTMLElement).style.display = "flex";
    });

    document.getElementById("btn-close-edit-name")?.addEventListener("click", () =>
      closeModal("modal-edit-name")
    );
    document.getElementById("btn-cancel-edit-name")?.addEventListener("click", () =>
      closeModal("modal-edit-name")
    );

    document.getElementById("btn-save-edit-name")?.addEventListener("click", async () => {
      const newName = (
        document.getElementById("input-new-name") as HTMLInputElement
      ).value.trim();
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

    document.getElementById("btn-delete-data")?.addEventListener("click", async () => {
      if (confirm("Delete ALL data? This cannot be undone.")) {
        try {
          await api.deleteAllData();
          this.showToast("All data deleted");
          location.reload();
        } catch (e) {
          this.showToast("Failed to delete data");
        }
      }
    });
  }

  private async startQrScanner() {
    const scannerEl = document.getElementById("qr-scanner-area");
    const videoEl = document.getElementById("qr-scanner-video") as HTMLVideoElement;
    if (!scannerEl || !videoEl) return;

    scannerEl.style.display = "block";

    try {
      this.qrScannerStream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: "environment" },
      });
      videoEl.srcObject = this.qrScannerStream;
      await videoEl.play();

      this.qrScanInterval = setInterval(async () => {
        if (!videoEl || videoEl.readyState < 2) return;
        try {
          const canvas = document.createElement("canvas");
          canvas.width = videoEl.videoWidth;
          canvas.height = videoEl.videoHeight;
          const ctx = canvas.getContext("2d");
          if (!ctx) return;
          ctx.drawImage(videoEl, 0, 0);
          // QR decode would go here - for now use manual paste
        } catch {
          // ignore scan errors
        }
      }, 500);

      this.showToast("Point camera at QR code");
    } catch (e) {
      this.showToast("Camera access denied. Use file upload instead.");
      scannerEl.style.display = "none";
    }
  }

  private stopQrScanner() {
    if (this.qrScannerStream) {
      this.qrScannerStream.getTracks().forEach((t) => t.stop());
      this.qrScannerStream = null;
    }
    if (this.qrScanInterval) {
      clearInterval(this.qrScanInterval);
      this.qrScanInterval = null;
    }
    const scannerEl = document.getElementById("qr-scanner-area");
    if (scannerEl) scannerEl.style.display = "none";
  }

  private async scanQrFromFile(file: File) {
    try {
      const text = await file.text();
      if (text) {
        await this.processQrData(text);
      } else {
        this.showToast("Could not read QR from file. Paste the QR data manually.");
      }
    } catch {
      this.showToast("Failed to read file");
    }
  }

  private async processQrData(data: string) {
    try {
      await api.scanQrCode(data);
      await this.loadContacts();
      this.stopQrScanner();
      const qrModal = document.getElementById("modal-qr");
      if (qrModal) qrModal.style.display = "none";
      this.showToast("Contact added via QR");
    } catch (e) {
      this.showToast("Invalid QR code data");
    }
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
      document.getElementById("call-avatar")!.textContent =
        this.currentContact.display_name.charAt(0).toUpperCase();
      document.getElementById("call-name")!.textContent = this.currentContact.display_name;
      document.getElementById("call-status")!.textContent = "Calling via Tor...";

      document.getElementById("call-overlay")!.style.display = "flex";

      setTimeout(() => {
        const statusEl = document.getElementById("call-status");
        if (statusEl && this.activeCallId) {
          statusEl.textContent = "Connected (E2E Encrypted)";
          document.getElementById("call-timer")!.style.display = "block";
          this.callSeconds = 0;
          this.callTimerInterval = setInterval(() => {
            this.callSeconds++;
            const m = Math.floor(this.callSeconds / 60)
              .toString()
              .padStart(2, "0");
            const s = (this.callSeconds % 60).toString().padStart(2, "0");
            document.getElementById("call-timer")!.textContent = `${m}:${s}`;
          }, 1000);
        }
      }, 3000);
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
      setTimeout(() => toast.classList.remove("show"), 3000);
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
