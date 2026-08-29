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
  verified: boolean;
  blocked: boolean;
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
  reply_to: string | null;
  delivered: boolean;
  read: boolean;
  expires_at: number | null;
}

export interface Reaction {
  id: string;
  message_id: string;
  sender: string;
  emoji: string;
  timestamp: number;
}

export interface TypingStatus {
  peer_onion: string;
  is_typing: boolean;
  last_typing_at: number;
}

export interface Group {
  id: string;
  name: string;
  description: string;
  created_by: string;
  created_at: number;
  member_count: number;
}

export interface GroupMember {
  group_id: string;
  onion_address: string;
  public_key: string;
  display_name: string;
  role: string;
  joined_at: number;
}

export interface GroupMessage {
  id: string;
  group_id: string;
  sender: string;
  content: string;
  timestamp: number;
  message_type: string;
  reply_to: string | null;
}

export interface CallLogEntry {
  id: string;
  peer_onion: string;
  call_type: string;
  direction: string;
  started_at: number;
  ended_at: number | null;
  duration_secs: number | null;
  status: string;
}

export interface FileTransfer {
  id: string;
  sender: string;
  recipient: string;
  filename: string;
  mime_type: string;
  size: number;
  status: string;
  started_at: number;
  completed_at: number | null;
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

export interface AppSettings {
  disappearing_messages_default: boolean;
  default_ttl_secs: number;
  read_receipts: boolean;
  typing_indicators: boolean;
  notifications_enabled: boolean;
  theme: string;
}

export const api = {
  // ── Identity ────────────────────────────────────────────────────────────
  initIdentity: (displayName: string): Promise<Identity> =>
    invoke("init_identity", { displayName }),

  getIdentity: (): Promise<Identity> =>
    invoke("get_identity"),

  getOnionAddress: (): Promise<string> =>
    invoke("get_onion_address"),

  // ── Contacts ────────────────────────────────────────────────────────────
  addContact: (displayName: string, publicKey: string, onionAddress: string): Promise<Contact> =>
    invoke("add_contact", { displayName, publicKey, onionAddress }),

  getContacts: (): Promise<Contact[]> =>
    invoke("get_contacts"),

  getContact: (onionAddress: string): Promise<Contact> =>
    invoke("get_contact", { onionAddress }),

  deleteContact: (onionAddress: string): Promise<void> =>
    invoke("delete_contact", { onionAddress }),

  blockContact: (onionAddress: string): Promise<void> =>
    invoke("block_contact", { onionAddress }),

  unblockContact: (onionAddress: string): Promise<void> =>
    invoke("unblock_contact", { onionAddress }),

  verifyContact: (onionAddress: string): Promise<void> =>
    invoke("verify_contact", { onionAddress }),

  // ── Messages ────────────────────────────────────────────────────────────
  sendMessage: (recipientOnion: string, content: string, messageType: string): Promise<Message> =>
    invoke("send_message", { recipientOnion, content, messageType }),

  sendReplyMessage: (recipientOnion: string, content: string, messageType: string, replyTo: string): Promise<Message> =>
    invoke("send_reply_message", { recipientOnion, content, messageType, replyTo }),

  getMessages: (contactOnion: string): Promise<Message[]> =>
    invoke("get_messages", { contactOnion }),

  deleteMessage: (messageId: string): Promise<void> =>
    invoke("delete_message", { messageId }),

  searchMessages: (query: string): Promise<Message[]> =>
    invoke("search_messages", { query }),

  markMessagesRead: (contactOnion: string): Promise<void> =>
    invoke("mark_messages_read", { contactOnion }),

  setDisappearingMessage: (messageId: string, ttlSecs: number): Promise<void> =>
    invoke("set_disappearing_message", { messageId, ttlSecs }),

  // ── Reactions ───────────────────────────────────────────────────────────
  addReaction: (messageId: string, emoji: string): Promise<Reaction> =>
    invoke("add_reaction", { messageId, emoji }),

  removeReaction: (messageId: string, emoji: string): Promise<void> =>
    invoke("remove_reaction", { messageId, emoji }),

  getReactions: (messageId: string): Promise<Reaction[]> =>
    invoke("get_reactions", { messageId }),

  // ── Typing ──────────────────────────────────────────────────────────────
  sendTypingIndicator: (peerOnion: string, isTyping: boolean): Promise<void> =>
    invoke("send_typing_indicator", { peerOnion, isTyping }),

  getTypingStatus: (peerOnion: string): Promise<TypingStatus> =>
    invoke("get_typing_status", { peerOnion }),

  // ── Groups ──────────────────────────────────────────────────────────────
  createGroup: (name: string, description: string): Promise<Group> =>
    invoke("create_group", { name, description }),

  getGroups: (): Promise<Group[]> =>
    invoke("get_groups"),

  getGroup: (groupId: string): Promise<Group> =>
    invoke("get_group", { groupId }),

  addGroupMember: (groupId: string, displayName: string, publicKey: string, onionAddress: string): Promise<GroupMember> =>
    invoke("add_group_member", { groupId, displayName, publicKey, onionAddress }),

  removeGroupMember: (groupId: string, onionAddress: string): Promise<void> =>
    invoke("remove_group_member", { groupId, onionAddress }),

  sendGroupMessage: (groupId: string, content: string, messageType: string): Promise<GroupMessage> =>
    invoke("send_group_message", { groupId, content, messageType }),

  getGroupMessages: (groupId: string): Promise<GroupMessage[]> =>
    invoke("get_group_messages", { groupId }),

  getGroupMembers: (groupId: string): Promise<GroupMember[]> =>
    invoke("get_group_members", { groupId }),

  // ── Calls ───────────────────────────────────────────────────────────────
  startVideoCall: (recipientOnion: string): Promise<string> =>
    invoke("start_video_call", { recipientOnion }),

  startAudioCall: (recipientOnion: string): Promise<string> =>
    invoke("start_audio_call", { recipientOnion }),

  answerVideoCall: (callId: string): Promise<void> =>
    invoke("answer_video_call", { callId }),

  rejectCall: (callId: string): Promise<void> =>
    invoke("reject_call", { callId }),

  createIncomingCall: (callId: string, peerOnion: string, callType: string): Promise<void> =>
    invoke("create_incoming_call", { callId, peerOnion, callType }),

  sendVoicePacket: (toOnion: string, callId: string, seq: number, data: number[], sampleRate: number, channels: number): Promise<void> =>
    invoke("send_voice_packet", { ofOnion: toOnion, callId, seq, data, sampleRate, channels }),

  sendVideoFrame: (toOnion: string, callId: string, seq: number, data: number[], width: number, height: number): Promise<void> =>
    invoke("send_video_frame", { ofOnion: toOnion, callId, seq, data, width, height }),

  sendScreenFrame: (toOnion: string, callId: string, seq: number, data: number[], width: number, height: number): Promise<void> =>
    invoke("send_screen_frame", { ofOnion: toOnion, callId, seq, data, width, height }),

  endVideoCall: (callId: string): Promise<void> =>
    invoke("end_video_call", { callId }),

  startScreenShare: (callId: string): Promise<void> =>
    invoke("start_screen_share", { callId }),

  stopScreenShare: (callId: string): Promise<void> =>
    invoke("stop_screen_share", { callId }),

  toggleAudioMute: (callId: string): Promise<void> =>
    invoke("toggle_audio_mute", { callId }),

  toggleVideo: (callId: string): Promise<void> =>
    invoke("toggle_video", { callId }),

  getCallHistory: (): Promise<CallLogEntry[]> =>
    invoke("get_call_history"),

  getActiveCalls: (): Promise<any[]> =>
    invoke("get_active_calls"),

  // ── Files ───────────────────────────────────────────────────────────────
  sendFile: (recipientOnion: string, fileData: string, fileName: string, mimeType: string): Promise<FileTransfer> =>
    invoke("send_file", { recipientOnion, fileData, fileName, mimeType }),

  getFileTransfers: (): Promise<FileTransfer[]> =>
    invoke("get_file_transfers"),

  // ── Voice Notes ──────────────────────────────────────────────────────
  sendVoiceNote: (recipientOnion: string, fileData: string, fileName: string, mimeType: string, durationSecs: number): Promise<Message> =>
    invoke("send_voice_note", { recipientOnion, fileData, fileName, mimeType, durationSecs }),

  // ── Forwards ─────────────────────────────────────────────────────────
  sendForwardMessage: (recipientOnion: string, originalSender: string, originalContent: string): Promise<Message> =>
    invoke("send_forward_message", { recipientOnion, originalSender, originalContent }),

  // ── QR ──────────────────────────────────────────────────────────────────
  generateQrCode: (): Promise<string> =>
    invoke("generate_qr_code"),

  scanQrCode: (qrData: string): Promise<Contact> =>
    invoke("scan_qr_code", { qrData }),

  // ── Tor ─────────────────────────────────────────────────────────────────
  getTorStatus: (): Promise<TorStatus> =>
    invoke("get_tor_status"),

  // ── Security ────────────────────────────────────────────────────────────
  getEncryptionInfo: (): Promise<EncryptionInfo> =>
    invoke("get_encryption_info"),

  deleteAllData: (): Promise<void> =>
    invoke("delete_all_data"),

  // ── Settings ────────────────────────────────────────────────────────────
  getSettings: (): Promise<AppSettings> =>
    invoke("get_settings"),

  updateSettings: (settings: Partial<AppSettings>): Promise<AppSettings> =>
    invoke("update_settings", { settings }),
};
