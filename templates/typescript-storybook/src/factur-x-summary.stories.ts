// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import type { Meta, StoryObj } from "@storybook/html";
import { compileTemplate } from "../../typescript/src/index.ts";
import { template, data } from "../../typescript/examples/factur-x-summary.ts";
import { renderStory, type StoryRenderArgs } from "./render.ts";

const meta: Meta<StoryRenderArgs> = {
  title: "Templates/Factur-X Summary",
  render: renderStory,
};
export default meta;

type Story = StoryObj<StoryRenderArgs>;

export const Base: Story = {
  args: {
    templateName: "factur-x-summary",
    variant: "Base",
    description: "Factur-X profile summary block embedded in the PDF/A-3 visual.",
    typstSource: compileTemplate(template, data),
  },
};
