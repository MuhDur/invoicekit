// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

using System;
using SAPbouiCOM;

namespace InvoiceKit.B1Addon;

public sealed class InvoiceKitApplication
{
    private const string MenuUid = "INVOICEKIT_SEND";

    private readonly Application application;
    private readonly InvoiceKitSidecarClient sidecarClient;

    public InvoiceKitApplication(Application application, InvoiceKitSidecarClient sidecarClient)
    {
        this.application = application ?? throw new ArgumentNullException(nameof(application));
        this.sidecarClient = sidecarClient ?? throw new ArgumentNullException(nameof(sidecarClient));
    }

    public void Start()
    {
        EnsureMenu();
        application.MenuEvent += OnMenuEvent;
        application.StatusBar.SetText(
            "InvoiceKit add-on started.",
            BoMessageTime.bmt_Short,
            BoStatusBarMessageType.smt_Success);
    }

    private void EnsureMenu()
    {
        if (application.Menus.Exists(MenuUid))
        {
            return;
        }

        MenuCreationParams menuParams = (MenuCreationParams)application.CreateObject(BoCreatableObjectType.cot_MenuCreationParams);
        menuParams.Type = BoMenuType.mt_STRING;
        menuParams.UniqueID = MenuUid;
        menuParams.String = "Send via InvoiceKit";
        menuParams.Enabled = true;
        menuParams.Position = 20;

        application.Menus.Item("1280").SubMenus.AddEx(menuParams);
    }

    private void OnMenuEvent(ref MenuEvent eventArgs, out bool bubbleEvent)
    {
        bubbleEvent = true;

        if (eventArgs.BeforeAction || eventArgs.MenuUID != MenuUid)
        {
            return;
        }

        try
        {
            TransmitActiveInvoice();
        }
        catch (Exception ex)
        {
            application.StatusBar.SetText(
                ex.Message,
                BoMessageTime.bmt_Long,
                BoStatusBarMessageType.smt_Error);
        }
    }

    private void TransmitActiveInvoice()
    {
        SapInvoiceSnapshot snapshot = SapInvoiceSnapshot.FromActiveForm(application);
        InvoiceKitReceipt receipt = sidecarClient.Transmit(snapshot);

        application.StatusBar.SetText(
            $"InvoiceKit submission {receipt.SubmissionId} is {receipt.State}.",
            BoMessageTime.bmt_Long,
            BoStatusBarMessageType.smt_Success);
    }
}
