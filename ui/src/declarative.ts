/**
 * Public declarative templating engine shared by all official and user-created themes.
 *
 * Themes supply pure HTML + CSS (zero JavaScript). This runtime binds live browser
 * data, i18n strings, repeated lists, drag-and-drop reordering, and actions.
 */

import { t } from "./i18n";

export interface RepeatItem {
  id?: string | number;
  state?: string;
  [key: string]: unknown;
}

export interface ActionContext {
  action: string;
  param?: string;
  item?: RepeatItem;
  index?: number;
  event: Event;
}

export interface DeclarativeCallbacks {
  onAction: (ctx: ActionContext) => void;
  onSubmit?: (name: string, value: string) => void;
  onStartDragWindow?: () => void;
  onMaximizeWindow?: () => void;
  onReorder?: (collection: string, draggedId: string, toIndex: number) => void;
}

export class DeclarativeRuntime {
  private root: HTMLElement;
  private callbacks: DeclarativeCallbacks;
  private templates = new Map<string, { tpl: HTMLTemplateElement; parent: HTMLElement }>();
  private draggedCollection: string | null = null;
  private draggedItemId: string | null = null;

  constructor(root: HTMLElement, callbacks: DeclarativeCallbacks) {
    this.root = root;
    this.callbacks = callbacks;
  }

  applyStyles(tokensCss: string, themeCss: string): void {
    setOrCreateStyle("seu-tokens-css", tokensCss);
    setOrCreateStyle("seu-theme-css", themeCss);
  }

  mount(sanitizedHtml: string): void {
    this.templates.clear();
    this.root.innerHTML = sanitizedHtml;

    // Discover and preserve all <template data-seu-repeat="..."> slots.
    const tpls = this.root.querySelectorAll<HTMLTemplateElement>("template[data-seu-repeat]");
    for (const tpl of tpls) {
      const name = tpl.dataset.seuRepeat;
      const parent = tpl.parentElement;
      if (name && parent) {
        this.templates.set(name, { tpl, parent });
      }
    }

    this.wireStaticHandlers();
  }

  render(data: Record<string, unknown>, lists: Record<string, RepeatItem[]>): void {
    // 1. Update static elements (skipping template contents).
    this.applyBindingsToSubtree(this.root, data, undefined, undefined);

    // 2. Render each repeated collection cleanly inside its container.
    for (const [name, entry] of this.templates.entries()) {
      const items = lists[name] ?? [];
      const { tpl, parent } = entry;

      // Remove previous rendered siblings while keeping the <template>.
      for (const child of Array.from(parent.children)) {
        if (child !== tpl) {
          child.remove();
        }
      }

      const enableDrag = name === "tabs" || name === "shortcuts";

      items.forEach((item, index) => {
        const fragment = tpl.content.cloneNode(true) as DocumentFragment;
        const firstEl = fragment.firstElementChild as HTMLElement | null;
        if (!firstEl) return;

        if (item.state) {
          firstEl.setAttribute("data-state", item.state);
        }
        if (item.id !== undefined) {
          firstEl.dataset.itemId = String(item.id);
        }

        this.applyBindingsToSubtree(firstEl, data, item, index);
        this.wireItemHandlers(firstEl, name, item, index, enableDrag);

        parent.appendChild(firstEl);
      });
    }
  }

  setInputValue(inputName: string, value: string): void {
    const input = this.root.querySelector<HTMLInputElement>(
      `[data-seu-input="${CSS.escape(inputName)}"]`,
    );
    if (input && document.activeElement !== input) {
      input.value = value;
    }
  }

  private wireStaticHandlers(): void {
    for (const dragEl of this.root.querySelectorAll<HTMLElement>("[data-seu-drag-region]")) {
      dragEl.addEventListener("mousedown", (ev) => {
        if (ev.button === 0 && ev.detail === 1) {
          this.callbacks.onStartDragWindow?.();
        }
      });
      dragEl.addEventListener("dblclick", () => {
        this.callbacks.onMaximizeWindow?.();
      });
    }

    for (const form of this.root.querySelectorAll<HTMLFormElement>("form[data-seu-submit]")) {
      const submitName = form.dataset.seuSubmit ?? "";
      form.addEventListener("submit", (ev) => {
        ev.preventDefault();
        const input = form.querySelector<HTMLInputElement>("input[data-seu-input]");
        const value = input?.value ?? "";
        this.callbacks.onSubmit?.(submitName, value);
        input?.blur();
      });
    }

    // Wire static action buttons (outside <template>).
    for (const el of this.root.querySelectorAll<HTMLElement>("[data-seu-action]")) {
      if (el.closest("template")) continue;
      const action = el.dataset.seuAction ?? "";
      el.addEventListener("click", (event) => {
        event.stopPropagation();
        this.callbacks.onAction({
          action,
          param: el.dataset.seuParam,
          event,
        });
      });
    }
  }

  private wireItemHandlers(
    rootEl: HTMLElement,
    collection: string,
    item: RepeatItem,
    index: number,
    enableDrag: boolean,
  ): void {
    const allActionEls: HTMLElement[] = [];
    if (rootEl.hasAttribute("data-seu-action")) {
      allActionEls.push(rootEl);
    }
    allActionEls.push(...Array.from(rootEl.querySelectorAll<HTMLElement>("[data-seu-action]")));

    for (const el of allActionEls) {
      const action = el.dataset.seuAction ?? "";
      const rawParam = el.dataset.seuParam;
      const resolvedParam =
        rawParam && rawParam.startsWith("$item.")
          ? String(resolvePath(item, rawParam.slice(6)) ?? "")
          : rawParam;
      el.addEventListener("click", (event) => {
        if (el instanceof HTMLAnchorElement) {
          const mouseEv = event as MouseEvent;
          if (mouseEv.ctrlKey || mouseEv.metaKey || mouseEv.shiftKey || mouseEv.altKey) {
            return;
          }
          event.preventDefault();
        }
        event.stopPropagation();
        this.callbacks.onAction({
          action,
          param: resolvedParam,
          item,
          index,
          event,
        });
      });
    }

    const ctxAction = rootEl.dataset.seuContextAction;
    if (ctxAction) {
      rootEl.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        this.callbacks.onAction({ action: ctxAction, item, index, event });
      });
    }

    const midAction = rootEl.dataset.seuMiddleAction;
    if (midAction) {
      rootEl.addEventListener("auxclick", (event) => {
        if (event.button === 1) {
          event.preventDefault();
          this.callbacks.onAction({ action: midAction, item, index, event });
        }
      });
    }

    if (enableDrag && item.id !== undefined) {
      const idStr = String(item.id);
      rootEl.draggable = true;
      rootEl.addEventListener("dragstart", (ev) => {
        this.draggedCollection = collection;
        this.draggedItemId = idStr;
        ev.dataTransfer?.setData("text/plain", idStr);
      });
      rootEl.addEventListener("dragover", (ev) => {
        if (this.draggedCollection === collection) {
          ev.preventDefault();
          rootEl.classList.add("drag-over");
        }
      });
      rootEl.addEventListener("dragleave", () => {
        rootEl.classList.remove("drag-over");
      });
      rootEl.addEventListener("drop", (ev) => {
        ev.preventDefault();
        rootEl.classList.remove("drag-over");
        if (
          this.draggedCollection === collection &&
          this.draggedItemId !== null &&
          this.draggedItemId !== idStr
        ) {
          this.callbacks.onReorder?.(collection, this.draggedItemId, index);
        }
        this.draggedCollection = null;
        this.draggedItemId = null;
      });
    }
  }

  private applyBindingsToSubtree(
    container: HTMLElement,
    data: Record<string, unknown>,
    item?: RepeatItem,
    index?: number,
  ): void {
    const lookup = (expr: string): unknown => {
      if (expr === "$index") return (index ?? 0) + 1;
      if (expr.startsWith("$item.") && item) {
        return resolvePath(item, expr.slice(6));
      }
      return resolvePath(data, expr);
    };

    const elements: HTMLElement[] = [container];
    for (const node of container.querySelectorAll<HTMLElement>("*")) {
      if (!node.closest("template")) {
        elements.push(node);
      }
    }

    for (const el of elements) {
      const tKey = el.dataset.seuT;
      if (tKey) {
        el.textContent = t(tKey);
      }

      const titleKey = el.dataset.seuTitleT;
      if (titleKey) {
        el.title = t(titleKey);
      }

      const phKey = el.dataset.seuPlaceholderT;
      if (phKey && el instanceof HTMLInputElement) {
        el.placeholder = t(phKey);
      }

      const bindExpr = el.dataset.seuBind;
      if (bindExpr) {
        const raw = lookup(bindExpr);
        el.textContent = formatValue(raw, el.dataset.seuFormat);
      }

      const ifExpr = el.dataset.seuIf;
      if (ifExpr) {
        el.hidden = !Boolean(lookup(ifExpr));
      }

      const enabledExpr = el.dataset.seuDisabledUnless;
      if (enabledExpr && el instanceof HTMLButtonElement) {
        el.disabled = !Boolean(lookup(enabledExpr));
      }

      const hrefExpr = el.dataset.seuHref;
      if (hrefExpr && el instanceof HTMLAnchorElement) {
        const href = String(lookup(hrefExpr) ?? "#");
        if (
          href.startsWith("https://") ||
          href.startsWith("http://") ||
          href.startsWith("seu://") ||
          href === "#"
        ) {
          el.href = href;
        }
      }

      const bgExpr = el.dataset.seuStyleBg;
      if (bgExpr) {
        const color = String(lookup(bgExpr) ?? "");
        if (color) {
          el.style.background = color;
        }
      }

      const iconExpr = el.dataset.seuIcon;
      if (iconExpr) {
        const rawIcon = String(lookup(iconExpr) ?? "").trim();
        if (/^[a-z0-9 -]+$/i.test(rawIcon)) {
          if (el.dataset.seuBaseClass === undefined) {
            el.dataset.seuBaseClass = el.className;
          }
          el.className = `${el.dataset.seuBaseClass} ${rawIcon}`.trim();
        }
      }
    }
  }
}

function setOrCreateStyle(id: string, css: string): void {
  let styleEl = document.getElementById(id) as HTMLStyleElement | null;
  if (!styleEl) {
    styleEl = document.createElement("style");
    styleEl.id = id;
    document.head.appendChild(styleEl);
  }
  styleEl.textContent = css;
}

function resolvePath(obj: Record<string, unknown>, path: string): unknown {
  let cur: unknown = obj;
  for (const seg of path.split(".")) {
    if (cur === null || cur === undefined || typeof cur !== "object") {
      return undefined;
    }
    cur = (cur as Record<string, unknown>)[seg];
  }
  return cur;
}

function formatValue(value: unknown, format?: string): string {
  if (value === null || value === undefined) return "";
  if (format === "upper") {
    return String(value).toUpperCase();
  }
  if (format === "roman") {
    return toRoman(Number(value));
  }
  return String(value);
}

function toRoman(num: number): string {
  const table: [number, string][] = [
    [10, "X"],
    [9, "IX"],
    [5, "V"],
    [4, "IV"],
    [1, "I"],
  ];
  let n = Math.max(1, Math.floor(num));
  let out = "";
  for (const [val, sym] of table) {
    while (n >= val) {
      out += sym;
      n -= val;
    }
  }
  return out;
}
