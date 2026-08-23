import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "./Icon";
import { useI18n } from "../lib/i18n";

export type BatchNodeAction = "work" | "container" | "other" | "ignore" | "reset" | "tags" | "favorites" | "rematch";

interface BatchContextMenuProps {
  count: number;
  x: number;
  y: number;
  onAction: (action: BatchNodeAction) => void;
  onClose: () => void;
}

export function BatchContextMenu({ count, x, y, onAction, onClose }: BatchContextMenuProps) {
  const { number, t } = useI18n();
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: x, top: y });

  useEffect(() => {
    const close = () => onClose();
    const key = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", key);
    menuRef.current?.focus();
    return () => { window.removeEventListener("pointerdown", close); window.removeEventListener("keydown", key); };
  }, [onClose]);

  useLayoutEffect(() => {
    const placeMenu = () => {
      const menu = menuRef.current;
      if (!menu) return;
      const gutter = 8;
      const bounds = menu.getBoundingClientRect();
      setPosition({
        left: Math.max(gutter, Math.min(x, Math.max(gutter, window.innerWidth - bounds.width - gutter))),
        top: Math.max(gutter, Math.min(y, Math.max(gutter, window.innerHeight - bounds.height - gutter))),
      });
    };
    placeMenu();
    window.addEventListener("resize", placeMenu);
    return () => window.removeEventListener("resize", placeMenu);
  }, [x, y]);

  const action = (id: BatchNodeAction) => (event: React.MouseEvent) => {
    event.stopPropagation();
    onAction(id);
    onClose();
  };

  return (
    <div className="context-menu batch-context-menu" ref={menuRef} style={position} tabIndex={-1} onPointerDown={(event) => event.stopPropagation()}>
      <p>{t("selection.menuTitle", { count: number(count) })}</p>
      <button onClick={action("work")} type="button"><Icon name="work" />{t("menu.setWork")}</button>
      <button onClick={action("container")} type="button"><Icon name="folder" />{t("menu.setContainer")}</button>
      <button onClick={action("other")} type="button"><Icon name="archive" />{t("menu.setOtherResources")}</button>
      <button onClick={action("ignore")} type="button"><Icon name="close" />{t("selection.ignoreSelected")}</button>
      <button onClick={action("reset")} type="button"><Icon name="refresh" />{t("menu.reset")}</button>
      <span className="menu-divider" />
      <button onClick={action("tags")} type="button"><Icon name="tag" />{t("selection.applyTag")}</button>
      <button onClick={action("favorites")} type="button"><Icon name="bookmark" />{t("selection.addToFavorites")}</button>
      <button onClick={action("rematch")} type="button"><Icon name="bangumi" />{t("selection.rematchSelected")}</button>
    </div>
  );
}
