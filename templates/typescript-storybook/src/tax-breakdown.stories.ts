// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import type { Meta, StoryObj } from "@storybook/html";
import { compileTemplate } from "../../typescript/src/index.ts";
import { template, data } from "../../typescript/examples/tax-breakdown.ts";
import { reverseChargeData, withAllowanceData } from "./data.ts";
import { renderStory, type StoryRenderArgs } from "./render.ts";

const meta: Meta<StoryRenderArgs> = {
  title: "Templates/Tax Breakdown",
  render: renderStory,
};
export default meta;

type Story = StoryObj<StoryRenderArgs>;

export const Base: Story = {
  args: {
    templateName: "tax-breakdown",
    variant: "Base",
    description: "Default tax-breakdown rendering.",
    typstSource: compileTemplate(template, data),
  },
};

export const WithAllowance: Story = {
  args: {
    templateName: "tax-breakdown",
    variant: "With allowance",
    description: "Reduced taxable base after a -10% volume rebate.",
    typstSource: compileTemplate(template, withAllowanceData),
  },
};

export const ReverseCharge: Story = {
  args: {
    templateName: "tax-breakdown",
    variant: "Reverse charge (BR-AE)",
    description: "Reverse charge (BR-AE): zero VAT line; customer-side VAT due in destination MS.",
    typstSource: compileTemplate(template, reverseChargeData),
  },
};
