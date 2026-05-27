#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

"""Generate the CEN EN16931 CII D16B subset element-edge coverage artifact.

The generator reads the UN/CEFACT CII D16B CrossIndustryInvoice XSD bundle from
the CEN EN16931 validation repository and emits a deterministic JSON matrix.
Rows are schema element edges, not sample documents: every reachable
complexType/element pair in the EN16931 CII D16B subset under
rsm:CrossIndustryInvoice is classified.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, deque
from pathlib import Path
from xml.etree import ElementTree

XSD_NS = "{http://www.w3.org/2001/XMLSchema}"

SOURCE_REPOSITORY = "https://github.com/ConnectingEurope/eInvoicing-EN16931"
SOURCE_TAG = "validation-1.3.16"
SOURCE_COMMIT = "b6c9e06a59812fb1a83585da40923b3678a649ad"
SOURCE_SUBSET = "D16B SCRDM (Subset)/uncoupled clm/CII"

ROOT_SCHEMA = "CrossIndustryInvoice_100pD16B.xsd"
RAM_SCHEMA = "CrossIndustryInvoice_ReusableAggregateBusinessInformationEntity_100pD16B.xsd"
QDT_SCHEMA = "CrossIndustryInvoice_QualifiedDataType_100pD16B.xsd"
UDT_SCHEMA = "CrossIndustryInvoice_UnqualifiedDataType_100pD16B.xsd"

CII_DOCUMENT_FIELDS_EXTENSION_URN = "urn:invoicekit:cii:d16b:document-fields"
INVOICEKIT_CII_METADATA_EXTENSION_URN = "urn:invoicekit:cii:extension:metadata:v1"

CLASSES = {
    "current_ir",
    "invoicekit_metadata_extension",
    "cii_document_field_extension",
    "profile_extension_payload",
    "lossiness_ledger_preserved",
    "unsupported_gap",
}


def local_type(type_name: str | None) -> str:
    if not type_name:
        return ""
    return type_name.rsplit(":", 1)[-1]


def cardinality(element: ElementTree.Element) -> str:
    minimum = element.get("minOccurs", "1")
    maximum = element.get("maxOccurs", "1")
    return f"{minimum}..{maximum}"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_types(schema_root: Path) -> dict[str, ElementTree.Element]:
    types: dict[str, ElementTree.Element] = {}
    for filename in [ROOT_SCHEMA, RAM_SCHEMA]:
        root = ElementTree.parse(schema_root / filename).getroot()
        for complex_type in root.findall(f"{XSD_NS}complexType"):
            name = complex_type.get("name")
            if name:
                types[name] = complex_type
    return types


def reachable_types(types: dict[str, ElementTree.Element]) -> list[str]:
    ordered: list[str] = []
    seen: set[str] = set()
    queue: deque[str] = deque(["CrossIndustryInvoiceType"])
    while queue:
        type_name = queue.popleft()
        if type_name in seen:
            continue
        seen.add(type_name)
        if type_name not in types:
            continue
        ordered.append(type_name)
        for element in child_elements(types[type_name]):
            child_type = local_type(element.get("type"))
            if child_type in types and child_type not in seen:
                queue.append(child_type)
    return ordered


def child_elements(complex_type: ElementTree.Element) -> list[ElementTree.Element]:
    sequence = complex_type.find(f"{XSD_NS}sequence")
    if sequence is None:
        return []
    return list(sequence.findall(f"{XSD_NS}element"))


def classify(declaring_type: str, element: str, type_name: str) -> dict[str, object]:
    key = (declaring_type, element)

    document_fields = {
        ("HeaderTradeAgreementType", "BuyerReference"): (
            "buyer_reference",
            "BuyerReference is a buyer-assigned business reference; preserve it without using tenant_id.",
        ),
        ("ExchangedDocumentContextType", "BusinessProcessSpecifiedDocumentContextParameter"): (
            "business_process_context_ids[]",
            "Business-process context is repeatable CII business/profile data; preserve the IDs without using trace_id.",
        ),
    }
    if key in document_fields:
        field, strategy = document_fields[key]
        return row_class(
            "cii_document_field_extension",
            strategy,
            extension_fields=[
                f"CommercialDocument.extensions[{CII_DOCUMENT_FIELDS_EXTENSION_URN}].{field}"
            ],
        )

    if key == ("ExchangedDocumentContextType", "ApplicationSpecifiedDocumentContextParameter"):
        return row_class(
            "profile_extension_payload",
            "Application context is repeatable CII application/profile data; the InvoiceKit-owned metadata parameter is recorded as a named mapping decision.",
        )

    core = {
        ("CrossIndustryInvoiceType", "ExchangedDocument"): ["CommercialDocument.document_number"],
        ("CrossIndustryInvoiceType", "SupplyChainTradeTransaction"): ["CommercialDocument"],
        ("ExchangedDocumentType", "ID"): ["CommercialDocument.document_number", "CommercialDocument.id"],
        ("ExchangedDocumentType", "TypeCode"): ["CommercialDocument.document_type"],
        ("ExchangedDocumentType", "IssueDateTime"): ["CommercialDocument.issue_date"],
        ("ExchangedDocumentType", "IncludedNote"): ["CommercialDocument.notes[]"],
        ("NoteType", "Content"): ["CommercialDocument.notes[].text"],
        ("SupplyChainTradeTransactionType", "IncludedSupplyChainTradeLineItem"): [
            "CommercialDocument.lines[]"
        ],
        ("SupplyChainTradeTransactionType", "ApplicableHeaderTradeAgreement"): [
            "CommercialDocument.supplier",
            "CommercialDocument.customer",
        ],
        ("SupplyChainTradeTransactionType", "ApplicableHeaderTradeDelivery"): [
            "CommercialDocument.tax_point_date"
        ],
        ("SupplyChainTradeTransactionType", "ApplicableHeaderTradeSettlement"): [
            "CommercialDocument.currency",
            "CommercialDocument.payment_instructions",
            "CommercialDocument.tax_summary",
            "CommercialDocument.monetary_total",
        ],
        ("DocumentLineDocumentType", "LineID"): ["CommercialDocument.lines[].id"],
        ("SupplyChainTradeLineItemType", "AssociatedDocumentLineDocument"): [
            "CommercialDocument.lines[].id"
        ],
        ("SupplyChainTradeLineItemType", "SpecifiedTradeProduct"): [
            "CommercialDocument.lines[].description"
        ],
        ("SupplyChainTradeLineItemType", "SpecifiedLineTradeAgreement"): [
            "CommercialDocument.lines[].unit_price"
        ],
        ("SupplyChainTradeLineItemType", "SpecifiedLineTradeDelivery"): [
            "CommercialDocument.lines[].quantity",
            "CommercialDocument.lines[].unit_code",
        ],
        ("SupplyChainTradeLineItemType", "SpecifiedLineTradeSettlement"): [
            "CommercialDocument.lines[].line_extension_amount",
            "CommercialDocument.lines[].tax_category",
        ],
        ("TradeProductType", "Name"): ["CommercialDocument.lines[].description"],
        ("TradeProductType", "Description"): ["CommercialDocument.lines[].description"],
        ("LineTradeAgreementType", "NetPriceProductTradePrice"): [
            "CommercialDocument.lines[].unit_price"
        ],
        ("TradePriceType", "ChargeAmount"): ["CommercialDocument.lines[].unit_price"],
        ("LineTradeDeliveryType", "BilledQuantity"): ["CommercialDocument.lines[].quantity"],
        ("LineTradeDeliveryType", "CreditedQuantity"): ["CommercialDocument.lines[].quantity"],
        ("LineTradeSettlementType", "ApplicableTradeTax"): [
            "CommercialDocument.lines[].tax_category"
        ],
        ("LineTradeSettlementType", "SpecifiedTradeSettlementLineMonetarySummation"): [
            "CommercialDocument.lines[].line_extension_amount"
        ],
        ("TradeSettlementLineMonetarySummationType", "LineTotalAmount"): [
            "CommercialDocument.lines[].line_extension_amount"
        ],
        ("HeaderTradeAgreementType", "SellerTradeParty"): ["CommercialDocument.supplier"],
        ("HeaderTradeAgreementType", "BuyerTradeParty"): ["CommercialDocument.customer"],
        ("HeaderTradeDeliveryType", "ActualDeliverySupplyChainEvent"): [
            "CommercialDocument.tax_point_date"
        ],
        ("SupplyChainEventType", "OccurrenceDateTime"): ["CommercialDocument.tax_point_date"],
        ("HeaderTradeSettlementType", "PaymentReference"): [
            "CommercialDocument.payment_instructions[].reference"
        ],
        ("HeaderTradeSettlementType", "InvoiceCurrencyCode"): ["CommercialDocument.currency"],
        ("HeaderTradeSettlementType", "PayeeTradeParty"): ["CommercialDocument.payee"],
        ("HeaderTradeSettlementType", "SpecifiedTradeSettlementPaymentMeans"): [
            "CommercialDocument.payment_instructions[]"
        ],
        ("HeaderTradeSettlementType", "ApplicableTradeTax"): [
            "CommercialDocument.tax_summary[]"
        ],
        ("HeaderTradeSettlementType", "SpecifiedTradePaymentTerms"): [
            "CommercialDocument.payment_terms"
        ],
        ("HeaderTradeSettlementType", "SpecifiedTradeSettlementHeaderMonetarySummation"): [
            "CommercialDocument.monetary_total"
        ],
        ("TradeSettlementPaymentMeansType", "TypeCode"): [
            "CommercialDocument.payment_instructions[].kind"
        ],
        ("TradeSettlementPaymentMeansType", "PayeePartyCreditorFinancialAccount"): [
            "CommercialDocument.payment_instructions[].account"
        ],
        ("CreditorFinancialAccountType", "IBANID"): [
            "CommercialDocument.payment_instructions[].account"
        ],
        ("CreditorFinancialAccountType", "ProprietaryID"): [
            "CommercialDocument.payment_instructions[].account"
        ],
        ("TradePaymentTermsType", "Description"): ["CommercialDocument.payment_terms.description"],
        ("TradePaymentTermsType", "DueDateDateTime"): [
            "CommercialDocument.payment_terms.due_date",
            "CommercialDocument.due_date",
        ],
        ("TradeTaxType", "CalculatedAmount"): ["CommercialDocument.tax_summary[].tax_amount"],
        ("TradeTaxType", "BasisAmount"): ["CommercialDocument.tax_summary[].taxable_amount"],
        ("TradeTaxType", "CategoryCode"): ["CommercialDocument.tax_summary[].category_code"],
        ("TradeTaxType", "RateApplicablePercent"): ["CommercialDocument.tax_summary[].tax_rate"],
        ("TradeTaxType", "TypeCode"): ["CommercialDocument.tax_summary[]"],
        ("TradeSettlementHeaderMonetarySummationType", "LineTotalAmount"): [
            "CommercialDocument.monetary_total.line_extension_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "AllowanceTotalAmount"): [
            "CommercialDocument.monetary_total.allowance_total_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "ChargeTotalAmount"): [
            "CommercialDocument.monetary_total.charge_total_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "TaxBasisTotalAmount"): [
            "CommercialDocument.monetary_total.tax_exclusive_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "TaxTotalAmount"): [
            "CommercialDocument.monetary_total.tax_inclusive_amount - tax_exclusive_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "GrandTotalAmount"): [
            "CommercialDocument.monetary_total.tax_inclusive_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "TotalPrepaidAmount"): [
            "CommercialDocument.monetary_total.prepaid_amount"
        ],
        ("TradeSettlementHeaderMonetarySummationType", "DuePayableAmount"): [
            "CommercialDocument.monetary_total.payable_amount"
        ],
        ("TradePartyType", "Name"): ["Party.name"],
        ("TradePartyType", "SpecifiedLegalOrganization"): ["Party.id"],
        ("TradePartyType", "DefinedTradeContact"): ["Party.contact"],
        ("TradePartyType", "PostalTradeAddress"): ["Party.address"],
        ("TradePartyType", "SpecifiedTaxRegistration"): ["Party.tax_ids[]"],
        ("LegalOrganizationType", "ID"): ["Party.id"],
        ("TaxRegistrationType", "ID"): ["Party.tax_ids[].value"],
        ("TradeAddressType", "PostcodeCode"): ["PostalAddress.postal_code"],
        ("TradeAddressType", "LineOne"): ["PostalAddress.lines[0]"],
        ("TradeAddressType", "LineTwo"): ["PostalAddress.lines[1]"],
        ("TradeAddressType", "LineThree"): ["PostalAddress.lines[2..]"],
        ("TradeAddressType", "CityName"): ["PostalAddress.city"],
        ("TradeAddressType", "CountryID"): ["PostalAddress.country"],
        ("TradeAddressType", "CountrySubDivisionName"): ["PostalAddress.subdivision"],
        ("TradeContactType", "PersonName"): ["Contact.name"],
        ("TradeContactType", "TelephoneUniversalCommunication"): ["Contact.phone"],
        ("TradeContactType", "EmailURIUniversalCommunication"): ["Contact.email"],
        ("UniversalCommunicationType", "CompleteNumber"): ["Contact.phone"],
        ("UniversalCommunicationType", "URIID"): ["Contact.email"],
    }
    if key in core:
        return row_class(
            "current_ir",
            "Mapped by the current invoicekit-format-cii parser/serializer for the supported core subset.",
            current_ir_paths=core[key],
        )

    if key in {("TradePartyType", "ID"), ("TradePartyType", "GlobalID")}:
        return row_class(
            "lossiness_ledger_preserved",
            "CII party identifiers are repeatable; the current parser can collapse one value into Party.id, but full multiplicity and scheme data need preservation.",
            current_ir_paths=["Party.id"],
        )

    if key == ("CrossIndustryInvoiceType", "ExchangedDocumentContext"):
        return row_class(
            "profile_extension_payload",
            "Document context is the CII profile/application metadata container; child rows record the exact current mappings.",
        )

    profile_names = (
        "BIMSpecifiedDocumentContextParameter",
        "ScenarioSpecifiedDocumentContextParameter",
        "GuidelineSpecifiedDocumentContextParameter",
        "SubsetSpecifiedDocumentContextParameter",
        "MessageStandardSpecifiedDocumentContextParameter",
        "TestIndicator",
        "SpecifiedTransactionID",
    )
    if declaring_type == "ExchangedDocumentContextType" and (
        element in profile_names or element.endswith("DocumentContextParameter")
    ):
        return row_class(
            "profile_extension_payload",
            "Document-context/profile assertion belongs to profile-specific payload handling.",
        )

    lossiness_terms = (
        "ReferencedDocument",
        "Reference",
        "SpecifiedPeriod",
        "AllowanceCharge",
        "Delivery",
        "TradeDelivery",
        "SupplyChainEvent",
        "TradeProduct",
        "Product",
        "TaxRepresentative",
        "TradeParty",
        "AccountingAccount",
        "Logistics",
        "CurrencyExchange",
        "AdvancePayment",
        "FinancialAdjustment",
        "Marketplace",
        "ProcuringProject",
        "BinaryFile",
        "PaymentTerms",
    )
    if any(
        term in declaring_type or term in element or term in type_name for term in lossiness_terms
    ):
        return row_class(
            "lossiness_ledger_preserved",
            "Recognized CII business surface that needs explicit semantic fields or a lossiness-ledger preservation pass before full-fidelity claims.",
        )

    if "CurrencyCode" in element and element != "InvoiceCurrencyCode":
        return row_class(
            "unsupported_gap",
            "Currency specialization is outside the current single-document-currency IR.",
        )

    return row_class(
        "unsupported_gap",
        "No current IR field, profile-extension strategy, or lossiness-ledger preservation strategy is implemented yet.",
    )


def row_class(
    class_name: str,
    strategy: str,
    *,
    current_ir_paths: list[str] | None = None,
    extension_fields: list[str] | None = None,
) -> dict[str, object]:
    if class_name not in CLASSES:
        raise ValueError(f"unknown class {class_name}")
    return {
        "class": class_name,
        "current_ir_paths": current_ir_paths or [],
        "extension_fields": extension_fields or [],
        "strategy": strategy,
    }


def generate(schema_root: Path) -> dict[str, object]:
    types = load_types(schema_root)
    type_names = reachable_types(types)
    elements: list[dict[str, object]] = []
    for declaring_type in type_names:
        for child in child_elements(types[declaring_type]):
            element = child.get("name", "")
            type_name = child.get("type", "")
            classification = classify(declaring_type, element, local_type(type_name))
            elements.append(
                {
                    "declaring_type": declaring_type,
                    "element": element,
                    "type": type_name,
                    "cardinality": cardinality(child),
                    **classification,
                }
            )

    counts = Counter(row["class"] for row in elements)
    for class_name in CLASSES:
        counts.setdefault(class_name, 0)
    counts.update(
        {
            "complex_types_reachable": len(type_names),
            "elements_total": len(elements),
        }
    )

    return {
        "schema_version": 1,
        "generated_at": "2026-05-27",
        "source": {
            "repository": SOURCE_REPOSITORY,
            "tag": SOURCE_TAG,
            "tag_ref": f"refs/tags/{SOURCE_TAG}",
            "commit": SOURCE_COMMIT,
            "subset": SOURCE_SUBSET,
            "source_files": [
                {"path": ROOT_SCHEMA, "sha256": sha256_file(schema_root / ROOT_SCHEMA)},
                {"path": RAM_SCHEMA, "sha256": sha256_file(schema_root / RAM_SCHEMA)},
                {"path": QDT_SCHEMA, "sha256": sha256_file(schema_root / QDT_SCHEMA)},
                {"path": UDT_SCHEMA, "sha256": sha256_file(schema_root / UDT_SCHEMA)},
            ],
        },
        "classes": sorted(CLASSES),
        "counts": dict(sorted(counts.items())),
        "named_mapping_decisions": [
            {
                "element": "HeaderTradeAgreementType/BuyerReference",
                "class": "cii_document_field_extension",
                "representation": f"CommercialDocument.extensions[{CII_DOCUMENT_FIELDS_EXTENSION_URN}].buyer_reference",
                "rationale": "BuyerReference is the buyer-assigned business reference; it is never tenant_id.",
            },
            {
                "element": "ExchangedDocumentContextType/BusinessProcessSpecifiedDocumentContextParameter",
                "class": "cii_document_field_extension",
                "representation": f"CommercialDocument.extensions[{CII_DOCUMENT_FIELDS_EXTENSION_URN}].business_process_context_ids[]",
                "rationale": "Business-process context is repeatable and identifies business processes; it is never trace_id.",
            },
            {
                "element": (
                    "ExchangedDocumentContextType/"
                    "ApplicationSpecifiedDocumentContextParameter"
                    f"[ID={INVOICEKIT_CII_METADATA_EXTENSION_URN}]"
                ),
                "class": "invoicekit_metadata_extension",
                "representation": (
                    "CommercialDocument.meta via "
                    f"{INVOICEKIT_CII_METADATA_EXTENSION_URN}"
                ),
                "rationale": "Only InvoiceKit's own application context parameter carries tenant_id, trace_id, and source_system.",
            },
        ],
        "elements": elements,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    artifact = generate(args.schema_root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
