// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

package dev.invoicekit.examples.springboot;

import static org.assertj.core.api.Assertions.assertThat;
import static org.springframework.test.web.servlet.request.MockMvcRequestBuilders.get;
import static org.springframework.test.web.servlet.request.MockMvcRequestBuilders.post;
import static org.springframework.test.web.servlet.result.MockMvcResultMatchers.jsonPath;
import static org.springframework.test.web.servlet.result.MockMvcResultMatchers.status;

import org.junit.jupiter.api.Test;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.boot.test.autoconfigure.web.servlet.AutoConfigureMockMvc;
import org.springframework.boot.test.context.SpringBootTest;
import org.springframework.test.web.servlet.MockMvc;
import org.springframework.test.web.servlet.MvcResult;

/**
 * T-1403 smoke suite: hit every endpoint via Spring's MockMvc so
 * the gate stays fast and runs the same code path the server
 * would. Requires the cdylib to be on the path (INVOICEKIT_FFI_LIB
 * env var or one of the bindings/java default candidates).
 */
@SpringBootTest
@AutoConfigureMockMvc
class SmokeIT {

    @Autowired
    MockMvc mvc;

    @Test
    void rootListsFixtures() throws Exception {
        mvc.perform(get("/"))
            .andExpect(status().isOk())
            .andExpect(jsonPath("$.title").value("InvoiceKit Spring Boot demo"))
            .andExpect(jsonPath("$.fixtures").isArray());
    }

    @Test
    void healthz() throws Exception {
        mvc.perform(get("/healthz"))
            .andExpect(status().isOk())
            .andExpect(jsonPath("$.status").value("ok"));
    }

    @Test
    void canonicalizeBasicFixture() throws Exception {
        MvcResult result = mvc.perform(post("/canonicalize/basic"))
            .andExpect(status().isOk())
            .andReturn();
        assertThat(result.getResponse().getContentAsString())
            .contains("\"_engine_status\":0");
    }

    @Test
    void canonicalizeWithAllowanceFixture() throws Exception {
        mvc.perform(post("/canonicalize/with-allowance"))
            .andExpect(status().isOk())
            .andExpect(jsonPath("$._engine_status").value(0));
    }

    @Test
    void canonicalizeReverseChargeFixture() throws Exception {
        mvc.perform(post("/canonicalize/reverse-charge"))
            .andExpect(status().isOk())
            .andExpect(jsonPath("$._engine_status").value(0));
    }

    @Test
    void unknownFixtureReturns404() throws Exception {
        mvc.perform(post("/canonicalize/does-not-exist"))
            .andExpect(status().isNotFound())
            .andExpect(jsonPath("$.error.code").value("UNKNOWN_FIXTURE"));
    }
}
