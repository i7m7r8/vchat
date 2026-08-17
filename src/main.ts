import { api } from "./lib/api";
import { store } from "./lib/store";

class VchatApp {
  private currentContact: any = null;
  private activeCallId: string | null = null;

  async init() {
    await this.loadIdentity();
    await this.loadContacts();
    this.setupEventListeners();
  }

  private async loadIdentity() {
    try {
      let identity = await api.getIdentity();
      if (!identity) {
        identity = await api.initIdentity("User");
      }
      store.setIdentity(identity);
      this.updateOnionAddress(identity.onion_address);
    } catch (error) {
      console.error("Failed to load identity:", error);
    }
  }

  private updateOnionAddress(address: string) {
    const el = document.getElementById("onion-address");
    if (el) el.textContent = address;
  }

  private async loadContacts() {
    try {
      const contacts = await api.getContacts();
      store.setContacts(contacts);
      this.renderContacts(contacts);
    } catch (error) {
      console.error("Failed to load contacts:", error);
    }
  }

  private renderContacts(contacts: any[]) {
    const list = document.getElementById("contacts-list");
    if (!list) return;

    list.innerHTML = "";

    if (contacts.length === 0) {
      list.innerHTML = `
        <div class="empty-state" style="padding: 40px 20px;">
          <p style="font-size: 13px; color: var(--text-muted); text-align: center;">
            No contacts yet.<br>Click + to add someone.
          </p>
        </div>
      `;
      return;
    }

    contacts.forEach((contact) => {
      const item = document.createElement("div");
      item.className = "contact-item";
      item.dataset.onion = contact.onion_address;

      const initial = contact.display_name.charAt(0).toUpperCase();
      const shortOnion = contact.onion_address.substring(0, 16) + "...";

      item.innerHTML = `
        <div class="contact-avatar">${initial}</div>
        <div class="contact-info">
          <div class="contact-name">${this.escapeHtml(contact.display_name)}</div>
          <div class="contact-onion">${shortOnion}</div>
        </div>
      `;

      item.addEventListener("click", () => this.selectContact(contact));
      list.appendChild(item);
    });
  }

  private async selectContact(contact: any) {
    this.currentContact = contact;
    store.setSelectedContact(contact);

    document.querySelectorAll(".contact-item").forEach((el) => {
      const htmlEl = el as HTMLElement;
      htmlEl.classList.toggle("active", htmlEl.dataset.onion === contact.onion_address);
    });

    const chatName = document.getElementById("chat-name");
    const chatStatus = document.getElementById("chat-status");
    const messageArea = document.getElementById("message-input-area");
    const btnVideoCall = document.getElementById("btn-video-call") as HTMLButtonElement;
    const btnScreenShare = document.getElementById("btn-screen-share") as HTMLButtonElement;

    if (chatName) chatName.textContent = contact.display_name;
    if (chatStatus) chatStatus.textContent = contact.onion_address;
    if (messageArea) messageArea.style.display = "block";
    if (btnVideoCall) btnVideoCall.disabled = false;
    if (btnScreenShare) btnScreenShare.disabled = false;

    await this.loadMessages(contact.onion_address);
  }

  private async loadMessages(contactOnion: string) {
    try {
      const messages = await api.getMessages(contactOnion);
      store.setMessages(contactOnion, messages);
      this.renderMessages(messages);
    } catch (error) {
      console.error("Failed to load messages:", error);
    }
  }

  private renderMessages(messages: any[]) {
    const container = document.getElementById("messages-container");
    if (!container) return;

    container.innerHTML = "";

    if (messages.length === 0) {
      container.innerHTML = `
        <div class="empty-state">
          <p style="color: var(--text-muted);">No messages yet. Say hello!</p>
        </div>
      `;
      return;
    }

    const identity = store.getIdentity();
    const myAddress = identity?.onion_address;

    messages.forEach((msg) => {
      const isSent = msg.sender === myAddress;
      const time = new Date(msg.timestamp * 1000).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      });

      const el = document.createElement("div");
      el.className = `message ${isSent ? "sent" : "received"}`;
      el.innerHTML = `
        <div class="message-content">${this.escapeHtml(msg.content)}</div>
        <div class="message-time">${time}</div>
        ${msg.encrypted ? '<div class="message-encrypted">E2E Encrypted</div>' : ""}
      `;
      container.appendChild(el);
    });

    container.scrollTop = container.scrollHeight;
  }

  private async sendMessage() {
    const input = document.getElementById("message-input") as HTMLTextAreaElement;
    if (!input || !input.value.trim() || !this.currentContact) return;

    const content = input.value.trim();
    input.value = "";
    this.autoResizeInput(input);

    try {
      const message = await api.sendMessage(
        this.currentContact.onion_address,
        content,
        "Text"
      );

      store.addMessage(this.currentContact.onion_address, message);
      this.renderMessages(
        store.getMessages(this.currentContact.onion_address)
      );
    } catch (error) {
      console.error("Failed to send message:", error);
    }
  }

  private async startVideoCall() {
    if (!this.currentContact) return;

    try {
      const callId = await api.startVideoCall(this.currentContact.onion_address);
      this.activeCallId = callId;
      store.setActiveCall(callId);
      this.showVideoPanel(true);
    } catch (error) {
      console.error("Failed to start video call:", error);
    }
  }

  private async endVideoCall() {
    if (!this.activeCallId) return;

    try {
      await api.endVideoCall(this.activeCallId);
      this.activeCallId = null;
      store.setActiveCall(null);
      this.showVideoPanel(false);
    } catch (error) {
      console.error("Failed to end video call:", error);
    }
  }

  private showVideoPanel(show: boolean) {
    const panel = document.getElementById("video-panel");
    if (panel) panel.style.display = show ? "flex" : "none";
  }

  private showAddContactModal(show: boolean) {
    const modal = document.getElementById("add-contact-modal");
    if (modal) modal.style.display = show ? "flex" : "none";
  }

  private async addContact() {
    const nameInput = document.getElementById("contact-name") as HTMLInputElement;
    const onionInput = document.getElementById("contact-onion") as HTMLInputElement;
    const keyInput = document.getElementById("contact-key") as HTMLInputElement;

    if (!nameInput?.value || !onionInput?.value || !keyInput?.value) {
      alert("Please fill in all fields");
      return;
    }

    try {
      const contact = await api.addContact(
        nameInput.value,
        keyInput.value,
        onionInput.value
      );

      store.addContact(contact);
      this.renderContacts(store.getContacts());
      this.showAddContactModal(false);

      nameInput.value = "";
      onionInput.value = "";
      keyInput.value = "";
    } catch (error) {
      console.error("Failed to add contact:", error);
      alert("Failed to add contact: " + error);
    }
  }

  private autoResizeInput(textarea: HTMLTextAreaElement) {
    textarea.style.height = "auto";
    textarea.style.height = Math.min(textarea.scrollHeight, 120) + "px";
  }

  private escapeHtml(text: string): string {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
  }

  private setupEventListeners() {
    document.getElementById("btn-add-contact")?.addEventListener("click", () => {
      this.showAddContactModal(true);
    });

    document.getElementById("btn-cancel-contact")?.addEventListener("click", () => {
      this.showAddContactModal(false);
    });

    document.getElementById("btn-save-contact")?.addEventListener("click", () => {
      this.addContact();
    });

    document.getElementById("btn-send")?.addEventListener("click", () => {
      this.sendMessage();
    });

    document.getElementById("message-input")?.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        this.sendMessage();
      }
    });

    document.getElementById("message-input")?.addEventListener("input", (e) => {
      this.autoResizeInput(e.target as HTMLTextAreaElement);
    });

    document.getElementById("btn-video-call")?.addEventListener("click", () => {
      this.startVideoCall();
    });

    document.getElementById("btn-end-call")?.addEventListener("click", () => {
      this.endVideoCall();
    });

    document.getElementById("btn-close-qr")?.addEventListener("click", () => {
      const modal = document.getElementById("qr-modal");
      if (modal) modal.style.display = "none";
    });
  }
}

const app = new VchatApp();
app.init();
