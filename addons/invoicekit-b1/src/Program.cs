// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System;
using SAPbouiCOM;

namespace InvoiceKit.B1Addon;

public static class Program
{
    public static int Main(string[] args)
    {
        if (args.Length == 0 || string.IsNullOrWhiteSpace(args[0]))
        {
            Console.Error.WriteLine("SAP Business One connection context argument is required.");
            return 64;
        }

        SboGuiApi guiApi = new();
        guiApi.Connect(args[0]);

        Application application = guiApi.GetApplication(-1);
        using InvoiceKitSidecarClient sidecarClient = InvoiceKitSidecarClient.FromEnvironment();

        InvoiceKitApplication invoiceKit = new(application, sidecarClient);
        invoiceKit.Start();

        System.Windows.Forms.Application.Run();
        return 0;
    }
}
