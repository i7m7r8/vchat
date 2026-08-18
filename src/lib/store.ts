import { Identity, Contact, Message } from "./api";

class Store {
  private static instance: Store;
  private identity: Identity | null = null;
  private contacts: Contact[] = [];
  private messages: Map<string, Message[]> = new Map();
  private selectedContact: Contact | null = null;
  private activeCall: string | null = null;

  static getInstance(): Store {
    if (!Store.instance) {
      Store.instance = new Store();
    }
    return Store.instance;
  }

  setIdentity(identity: Identity) {
    this.identity = identity;
  }

  getIdentity(): Identity | null {
    return this.identity;
  }

  setContacts(contacts: Contact[]) {
    this.contacts = contacts;
  }

  getContacts(): Contact[] {
    return this.contacts;
  }

  addContact(contact: Contact) {
    this.contacts.unshift(contact);
  }

  setMessages(contactOnion: string, messages: Message[]) {
    this.messages.set(contactOnion, messages);
  }

  getMessages(contactOnion: string): Message[] {
    return this.messages.get(contactOnion) || [];
  }

  addMessage(contactOnion: string, message: Message) {
    const msgs = this.messages.get(contactOnion) || [];
    msgs.push(message);
    this.messages.set(contactOnion, msgs);
  }

  setSelectedContact(contact: Contact) {
    this.selectedContact = contact;
  }

  getSelectedContact(): Contact | null {
    return this.selectedContact;
  }

  setActiveCall(callId: string | null) {
    this.activeCall = callId;
  }

  getActiveCall(): string | null {
    return this.activeCall;
  }
}

export const store = Store.getInstance();
