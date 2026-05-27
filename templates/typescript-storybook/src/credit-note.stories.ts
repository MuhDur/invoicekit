// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import type { Meta, StoryObj } from "@storybook/html";
import { compileTemplate } from "../../typescript/src/index.ts";
import { template, data } from "../../typescript/examples/credit-note.ts";
import { renderStory, type StoryRenderArgs } from "./render.ts";

const meta: Meta<StoryRenderArgs> = {
  title: "Templates/Credit Note",
  render: renderStory,
};
export default meta;

type Story = StoryObj<StoryRenderArgs>;

export const Base: Story = {
  args: {
    templateName: "credit-note",
    variant: "Base",
    description: "Default credit-note fixture from the templates package.",
    typstSource: compileTemplate(template, data),
  },
};
