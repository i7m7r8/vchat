import { invoke } from "@tauri-apps/api/core";

export interface Identity {
  public_key: string;
  onion_address: string;
  display_name: string;
}

export interface Contact {
  id: string;
  display_name: string;
  public_key: string;
  onion_address: string;
  added_at: number;
}

export interface Message {
  id: string;
  sender: string;
  recipient: string;
  content: string;
  timestamp: number;
  encrypted: boolean;
  message_type: string;
  sequence_num: number;
}

export interface TorStatus {
  connected: boolean;
  onion_address: string;
}

export interface EncryptionInfo {
  algorithm: string;
  key_exchange: string;
  key_derivation: string;
  signing: string;
  handshake: string;
  onion_version: string;
}

export const api = {
  async initIdentity(displayName: string): Promise<Identity> {
    return invoke("init_identity", { displayName });
  },

  async getIdentity(): Promise<Identity | null> {
    return invoke("get_identity");
  },

  async getOnionAddress(): Promise<string> {
    return invoke("get_onion_address");
  },

  async sendMessage(
    recipientOnion: string,
    content: string,
    messageType: string
  ): Promise<Message> {
    return invoke("send_message", {
      recipientOnion,
      content,
      messageType,
    });
  },

  async startVideoCall(recipientOnion: string): Promise<string> {
    return invoke("start_video_call", { recipientOnion });
  },

  async answerVideoCall(callId: string): Promise<void> {
    return invoke("answer_video_call", { callId });
  },

  async endVideoCall(callId: string): Promise<void> {
    return invoke("end_video_call", { callId });
  },

  async startScreenShare(callId: string): Promise<void> {
    return invoke("start_screen_share", { callId });
  },

  async stopScreenShare(callId: string): Promise<void> {
    return invoke("stop_screen_share", { callId });
  },

  async addContact(
    displayName: string,
    publicKey: string,
    onionAddress: string
  ): Promise<Contact> {
    return invoke("add_contact", {
      displayName,
      publicKey,
      onionAddress,
    });
  },

  async getContacts(): Promise<Contact[]> {
    return invoke("get_contacts");
  },

  async getMessages(contactOnion: string): Promise<Message[]> {
    return invoke("get_messages", { contactOnion });
  },

  async generateQrCode(): Promise<string> {
    return invoke("generate_qr_code");
  },

  async scanQrCode(qrData: string): Promise<Contact> {
    return invoke("scan_qr_code", { qrData });
  },

  async getTorStatus(): Promise<TorStatus> {
    return invoke("get_tor_status");
  },

  async deleteAllData(): Promise<void> {
    return invoke("delete_all_data");
  },

  async getEncryptionInfo(): Promise<EncryptionInfo> {
    return invoke("get_encryption_info");
  },
};
