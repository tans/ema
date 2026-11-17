import { useSyncExternalStore } from "react";
import { X } from "react-feather";
import "./toast.css";

type Toast = { id: number; text: string; tone: "info" | "error" };
let items: Toast[] = [];
let nextId = 0;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach(listener => listener());
function dismiss(id: number) {
  items = items.filter(item => item.id !== id);
  emit();
}
export function notify(text: string, tone: Toast["tone"] = "info") {
  if (!text.trim()) return;
  const id = ++nextId;
  items = [...items, { id, text, tone }].slice(-4);
  emit();
  window.setTimeout(() => dismiss(id), tone === "error" ? 7000 : 4000);
}
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
const snapshot = () => items;
export default function ToastHost({ language }: { language: "zh-CN" | "en" }) {
  const toasts = useSyncExternalStore(subscribe, snapshot);
  return <div className="ema-toasts" aria-label={language === "zh-CN" ? "操作反馈" : "Notifications"}>
    {toasts.map(toast => <div key={toast.id} className={`ema-toast ema-toast-${toast.tone}`} role={toast.tone === "error" ? "alert" : "status"}>
      <span>{toast.text}</span>
      <button type="button" onClick={() => dismiss(toast.id)} aria-label={language === "zh-CN" ? "关闭提示" : "Dismiss notification"}><X size={15}/></button>
    </div>)}
  </div>;
}
