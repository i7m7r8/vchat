class Store {
  private static instance: Store;
  private identity: any = null;
  private contacts: any[] = [];
  private messages: Map<string, any[]> = new Map();
  private selectedContact: any = null;
  private activeCall: string | null = null;

  static getInstance(): Store {
    if (!Store.instance) {
      Store.instance = new Store();
    }
    return Store.instance;
  }

  setIdentity(identity: any) {
    this.identity = identity;
  }

  getIdentity() {
    return this.identity;
  }

  setContacts(contacts: any[]) {
    this.contacts = contacts;
  }

  getContacts() {
    return this.contacts;
  }

  addContact(contact: any) {
    this.contacts.unshift(contact);
  }

  setMessages(contactOnion: string, messages: any[]) {
    this.messages.set(contactOnion, messages);
  }

  getMessages(contactOnion: string) {
    return this.messages.get(contactOnion) || [];
  }

  addMessage(contactOnion: string, message: any) {
    const msgs = this.messages.get(contactOnion) || [];
    msgs.push(message);
    this.messages.set(contactOnion, msgs);
  }

  setSelectedContact(contact: any) {
    this.selectedContact = contact;
  }

  getSelectedContact() {
    return this.selectedContact;
  }

  setActiveCall(callId: string | null) {
    this.activeCall = callId;
  }

  getActiveCall() {
    return this.activeCall;
  }
}

export const store = Store.getInstance();
