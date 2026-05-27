// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-114 helper: render a compiled Typst source string into a
// Storybook story HTMLElement with metadata + variant label.

export interface StoryRenderArgs {
  templateName: string;
  variant: string;
  description: string;
  typstSource: string;
}

export function renderStory(args: StoryRenderArgs): HTMLElement {
  const root = document.createElement("article");
  root.style.fontFamily = "system-ui, -apple-system, sans-serif";
  root.style.color = "#111";
  root.style.maxWidth = "900px";

  const title = document.createElement("h2");
  title.textContent = `${args.templateName} — ${args.variant}`;
  title.style.marginTop = "0";
  root.appendChild(title);

  const desc = document.createElement("p");
  desc.textContent = args.description;
  desc.style.color = "#444";
  root.appendChild(desc);

  const codeHeader = document.createElement("h3");
  codeHeader.textContent = "Compiled Typst source";
  codeHeader.style.marginBottom = "0.4rem";
  root.appendChild(codeHeader);

  const pre = document.createElement("pre");
  pre.style.background = "#f4f4f4";
  pre.style.padding = "1rem";
  pre.style.borderRadius = "6px";
  pre.style.overflow = "auto";
  pre.style.fontSize = "0.85rem";
  pre.textContent = args.typstSource;
  root.appendChild(pre);

  return root;
}
