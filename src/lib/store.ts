import type {
  Identity,
  Contact,
  Message,
  Reaction,
  TypingStatus,
  Group,
  GroupMessage,
  CallLogEntry,
  AppSettings,
} from "./api";

class Store {
  private static instance: Store;

  private identity: Identity | null = null;
  private contacts: Contact[] = [];
  private messages: Map<string, Message[]> = new Map();
  private groupMessages: Map<string, GroupMessage[]> = new Map();
  private groups: Group[] = [];
  private selectedContact: Contact | null = null;
  private selectedGroup: Group | null = null;
  private activeCall: string | null = null;
  private settings: AppSettings | null = null;
  private typingPeers: Map<string, TypingStatus> = new Map();
  private reactions: Map<string, Reaction[]> = new Map();
  private callHistory: CallLogEntry[] = [];
  private searchQuery: string = "";

  private listeners: Map<string, Set<() => void>> = new Map();

  private constructor() {}

  static getInstance(): Store {
    if (!Store.instance) {
      Store.instance = new Store();
    }
    return Store.instance;
  }

  // ── Listeners ───────────────────────────────────────────────────────────

  subscribe(key: string, callback: () => void): () => void {
    if (!this.listeners.has(key)) {
      this.listeners.set(key, new Set());
    }
    this.listeners.get(key)!.add(callback);
    return () => {
      this.listeners.get(key)?.delete(callback);
    };
  }

  private notify(key: string): void {
    this.listeners.get(key)?.forEach((cb) => cb());
    this.listeners.get("*")?.forEach((cb) => cb());
  }

  // ── Identity ────────────────────────────────────────────────────────────

  getIdentity(): Identity | null {
    return this.identity;
  }

  setIdentity(identity: Identity): void {
    this.identity = identity;
    this.notify("identity");
  }

  // ── Contacts ────────────────────────────────────────────────────────────

  getContacts(): Contact[] {
    return this.contacts;
  }

  setContacts(contacts: Contact[]): void {
    this.contacts = contacts;
    this.notify("contacts");
  }

  addContact(contact: Contact): void {
    this.contacts.push(contact);
    this.notify("contacts");
  }

  removeContact(onionAddress: string): void {
    this.contacts = this.contacts.filter((c) => c.onion_address !== onionAddress);
    if (this.selectedContact?.onion_address === onionAddress) {
      this.selectedContact = null;
      this.notify("selectedContact");
    }
    this.notify("contacts");
  }

  updateContact(onionAddress: string, updates: Partial<Contact>): void {
    const idx = this.contacts.findIndex((c) => c.onion_address === onionAddress);
    if (idx !== -1) {
      this.contacts[idx] = { ...this.contacts[idx], ...updates };
      if (this.selectedContact?.onion_address === onionAddress) {
        this.selectedContact = this.contacts[idx];
        this.notify("selectedContact");
      }
      this.notify("contacts");
    }
  }

  getContact(onionAddress: string): Contact | undefined {
    return this.contacts.find((c) => c.onion_address === onionAddress);
  }

  // ── Selected Contact ────────────────────────────────────────────────────

  getSelectedContact(): Contact | null {
    return this.selectedContact;
  }

  setSelectedContact(contact: Contact | null): void {
    this.selectedContact = contact;
    this.notify("selectedContact");
  }

  // ── Messages ────────────────────────────────────────────────────────────

  getMessagesForContact(contactOnion: string): Message[] {
    return this.messages.get(contactOnion) ?? [];
  }

  setMessagesForContact(contactOnion: string, msgs: Message[]): void {
    this.messages.set(contactOnion, msgs);
    this.notify("messages");
  }

  addMessage(contactOnion: string, message: Message): void {
    const list = this.messages.get(contactOnion) ?? [];
    list.push(message);
    this.messages.set(contactOnion, list);
    this.notify("messages");
  }

  removeMessage(messageId: string): void {
    for (const [key, msgs] of this.messages) {
      const filtered = msgs.filter((m) => m.id !== messageId);
      if (filtered.length !== msgs.length) {
        this.messages.set(key, filtered);
      }
    }
    this.reactions.delete(messageId);
    this.notify("messages");
    this.notify("reactions");
  }

  updateMessage(messageId: string, updates: Partial<Message>): void {
    for (const [key, msgs] of this.messages) {
      const idx = msgs.findIndex((m) => m.id === messageId);
      if (idx !== -1) {
        msgs[idx] = { ...msgs[idx], ...updates };
        this.notify("messages");
        return;
      }
    }
  }

  searchMessages(query: string): Message[] {
    const lower = query.toLowerCase();
    const results: Message[] = [];
    for (const msgs of this.messages.values()) {
      for (const msg of msgs) {
        if (msg.content.toLowerCase().includes(lower)) {
          results.push(msg);
        }
      }
    }
    return results.sort((a, b) => b.timestamp - a.timestamp);
  }

  markContactMessagesRead(contactOnion: string): void {
    const msgs = this.messages.get(contactOnion);
    if (msgs) {
      for (const msg of msgs) {
        msg.read = true;
        msg.delivered = true;
      }
      this.notify("messages");
    }
  }

  // ── Group Messages ──────────────────────────────────────────────────────

  getGroupMessagesForGroup(groupId: string): GroupMessage[] {
    return this.groupMessages.get(groupId) ?? [];
  }

  setGroupMessagesForGroup(groupId: string, msgs: GroupMessage[]): void {
    this.groupMessages.set(groupId, msgs);
    this.notify("groupMessages");
  }

  addGroupMessage(groupId: string, message: GroupMessage): void {
    const list = this.groupMessages.get(groupId) ?? [];
    list.push(message);
    this.groupMessages.set(groupId, list);
    this.notify("groupMessages");
  }

  // ── Groups ──────────────────────────────────────────────────────────────

  getGroups(): Group[] {
    return this.groups;
  }

  setGroups(groups: Group[]): void {
    this.groups = groups;
    this.notify("groups");
  }

  addGroup(group: Group): void {
    this.groups.push(group);
    this.notify("groups");
  }

  removeGroup(groupId: string): void {
    this.groups = this.groups.filter((g) => g.id !== groupId);
    this.groupMessages.delete(groupId);
    if (this.selectedGroup?.id === groupId) {
      this.selectedGroup = null;
      this.notify("selectedGroup");
    }
    this.notify("groups");
    this.notify("groupMessages");
  }

  updateGroup(groupId: string, updates: Partial<Group>): void {
    const idx = this.groups.findIndex((g) => g.id === groupId);
    if (idx !== -1) {
      this.groups[idx] = { ...this.groups[idx], ...updates };
      if (this.selectedGroup?.id === groupId) {
        this.selectedGroup = this.groups[idx];
        this.notify("selectedGroup");
      }
      this.notify("groups");
    }
  }

  getGroup(groupId: string): Group | undefined {
    return this.groups.find((g) => g.id === groupId);
  }

  // ── Selected Group ──────────────────────────────────────────────────────

  getSelectedGroup(): Group | null {
    return this.selectedGroup;
  }

  setSelectedGroup(group: Group | null): void {
    this.selectedGroup = group;
    this.notify("selectedGroup");
  }

  // ── Active Call ─────────────────────────────────────────────────────────

  getActiveCall(): string | null {
    return this.activeCall;
  }

  setActiveCall(callId: string | null): void {
    this.activeCall = callId;
    this.notify("activeCall");
  }

  // ── Settings ────────────────────────────────────────────────────────────

  getSettings(): AppSettings | null {
    return this.settings;
  }

  setSettings(settings: AppSettings): void {
    this.settings = settings;
    this.notify("settings");
  }

  updateSettings(partial: Partial<AppSettings>): void {
    if (this.settings) {
      this.settings = { ...this.settings, ...partial };
    }
    this.notify("settings");
  }

  // ── Typing ──────────────────────────────────────────────────────────────

  getTypingStatus(peerOnion: string): TypingStatus | undefined {
    return this.typingPeers.get(peerOnion);
  }

  setTypingStatus(status: TypingStatus): void {
    this.typingPeers.set(status.peer_onion, status);
    this.notify("typing");
  }

  getAllTypingPeers(): TypingStatus[] {
    return Array.from(this.typingPeers.values());
  }

  clearTypingStatus(peerOnion: string): void {
    this.typingPeers.delete(peerOnion);
    this.notify("typing");
  }

  // ── Reactions ───────────────────────────────────────────────────────────

  getReactionsForMessage(messageId: string): Reaction[] {
    return this.reactions.get(messageId) ?? [];
  }

  setReactionsForMessage(messageId: string, rxns: Reaction[]): void {
    this.reactions.set(messageId, rxns);
    this.notify("reactions");
  }

  addReaction(messageId: string, reaction: Reaction): void {
    const list = this.reactions.get(messageId) ?? [];
    list.push(reaction);
    this.reactions.set(messageId, list);
    this.notify("reactions");
  }

  removeReaction(messageId: string, sender: string, emoji: string): void {
    const list = this.reactions.get(messageId);
    if (list) {
      this.reactions.set(
        messageId,
        list.filter((r) => !(r.sender === sender && r.emoji === emoji))
      );
      this.notify("reactions");
    }
  }

  // ── Call History ────────────────────────────────────────────────────────

  getCallHistory(): CallLogEntry[] {
    return this.callHistory;
  }

  setCallHistory(history: CallLogEntry[]): void {
    this.callHistory = history;
    this.notify("callHistory");
  }

  addCallEntry(entry: CallLogEntry): void {
    this.callHistory.push(entry);
    this.notify("callHistory");
  }

  // ── Search ──────────────────────────────────────────────────────────────

  getSearchQuery(): string {
    return this.searchQuery;
  }

  setSearchQuery(query: string): void {
    this.searchQuery = query;
    this.notify("searchQuery");
  }

  // ── Reset ───────────────────────────────────────────────────────────────

  clearAll(): void {
    this.identity = null;
    this.contacts = [];
    this.messages.clear();
    this.groupMessages.clear();
    this.groups = [];
    this.selectedContact = null;
    this.selectedGroup = null;
    this.activeCall = null;
    this.settings = null;
    this.typingPeers.clear();
    this.reactions.clear();
    this.callHistory = [];
    this.searchQuery = "";
    this.notify("*");
  }
}

export const store = Store.getInstance();
