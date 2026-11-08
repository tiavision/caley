// `notify` actions: the nav rail's new-mail notification toggle (Notification API + its localStorage flag).
import {
  mailNotificationsEnabled,
  mailNotificationsStorageKey,
  mailNotificationsSupported,
} from '../lib/mail-notifications.ts';

export interface NotificationStatus {
  supported: boolean;
  permission: NotificationPermission;
  enabled: boolean;
}

/** Whether this browser can notify, the permission, and whether the user turned notifications on. */
export function status(): NotificationStatus {
  if (!mailNotificationsSupported()) return { supported: false, permission: 'default', enabled: false };
  return { supported: true, permission: Notification.permission, enabled: mailNotificationsEnabled() };
}

/** Turns notifications off, or asks for permission and turns them on; returns the new status. */
export async function toggle(current: NotificationStatus): Promise<NotificationStatus> {
  if (!current.supported || current.permission === 'denied') return current;
  if (current.enabled) {
    localStorage.removeItem(mailNotificationsStorageKey);
    return status();
  }
  const permission = Notification.permission === 'granted' ? 'granted' : await Notification.requestPermission();
  if (permission === 'granted') localStorage.setItem(mailNotificationsStorageKey, '1');
  return { ...status(), permission };
}
