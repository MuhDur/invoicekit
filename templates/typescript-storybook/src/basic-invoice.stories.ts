// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-114 stories for the basic-invoice template.

import type { Meta, StoryObj } from "@storybook/html";
import { compileTemplate } from "../../typescript/src/index.ts";
import { template } from "../../typescript/examples/basic-invoice.ts";
import { basicData, reverseChargeData, withAllowanceData } from "./data.ts";
import { renderStory, type StoryRenderArgs } from "./render.ts";

const meta: Meta<StoryRenderArgs> = {
  title: "Templates/Basic Invoice",
  render: renderStory,
};
export default meta;

type Story = StoryObj<StoryRenderArgs>;

export const Base: Story = {
  args: {
    templateName: "basic-invoice",
    variant: "Base",
    description: "Default Berlin → Munich invoice with two lines and 19% VAT.",
    typstSource: compileTemplate(template, basicData),
  },
};

export const WithAllowance: Story = {
  args: {
    templateName: "basic-invoice",
    variant: "With volume rebate",
    description: "Adds a -10% volume rebate line; net base drops to 135 EUR.",
    typstSource: compileTemplate(template, withAllowanceData),
  },
};

export const ReverseCharge: Story = {
  args: {
    templateName: "basic-invoice",
    variant: "Reverse charge (BR-AE)",
    description: "Reverse charge (BR-AE): customer in Austria; 0 EUR VAT due.",
    typstSource: compileTemplate(template, reverseChargeData),
  },
};
