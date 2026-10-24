//! Concept expansion for lexical matching.
//!
//! A profile term is a word the owner typed; a posting rarely uses that exact word. A memory-systems role may describe page migration across tiers and never write `DRAM`, and a platform firmware role may be titled after the component it runs on rather than after firmware. Matching on literal terms alone misses both, so each term is expanded to the concept it belongs to and the concept's other surface forms are matched too.
//!
//! This is a fixed vocabulary, not a model. Every match it produces can be explained by naming the concept and the phrase that hit, which is what makes the result auditable.

/// Concepts and the phrases that signal them. All phrases are stored in the normalized form produced by `normalize_location`: lowercase, umlauts expanded, punctuation collapsed to single spaces.
const CONCEPTS: &[(&str, &[&str])] = &[
    (
        "memory systems",
        &[
            "dram",
            "sdram",
            "ddr",
            "hbm",
            "lpddr",
            "cxl",
            "memory tiering",
            "tiered memory",
            "page migration",
            "memory disaggregation",
            "near memory",
            "processing in memory",
            "memory controller",
            "memory subsystem",
            "memory hierarchy",
            "numa",
            "cache coherence",
            "cache hierarchy",
            "memory bandwidth",
            "memory latency",
            "address translation",
            "virtual memory",
            "tlb",
            "huge pages",
            "memory allocator",
            "heterogeneous memory",
        ],
    ),
    (
        "firmware",
        &[
            "firmware",
            "rtkit",
            "aop",
            "smc",
            "pmu",
            "bootloader",
            "boot rom",
            "secure boot",
            "bare metal",
            "rtos",
            "embedded c",
            "device driver",
            "board bring up",
            "bring up",
            "power management",
            "dma",
            "interrupt handler",
            "hal",
            "bsp",
            "uefi",
            "coreboot",
            "zephyr",
            "freertos",
            "baseband",
            "microcontroller",
            "embedded software",
        ],
    ),
    (
        "computer architecture",
        &[
            "microarchitecture",
            "computer architecture",
            "cpu core",
            "out of order",
            "branch predictor",
            "instruction set",
            "risc v",
            "riscv",
            "arm architecture",
            "simulator",
            "gem5",
            "performance modeling",
            "performance architect",
            "soc architect",
            "chiplet",
            "interconnect",
            "noc",
            "accelerator",
            "systolic array",
            "dataflow",
        ],
    ),
    (
        "hardware security",
        &[
            "hardware security",
            "speculative execution",
            "transient execution",
            "spectre",
            "meltdown",
            "rowhammer",
            "read disturbance",
            "side channel",
            "fault injection",
            "microarchitectural security",
            "trusted execution",
            "tee",
            "secure enclave",
            "attestation",
            "cryptographic hardware",
            "root of trust",
        ],
    ),
    (
        "verification",
        &[
            "verification",
            "formal verification",
            "design verification",
            "uvm",
            "systemverilog",
            "assertion",
            "coverage closure",
            "testbench",
            "emulation",
            "post silicon",
            "pre silicon",
            "validation",
            "dft",
            "scan chain",
        ],
    ),
    (
        "hardware design",
        &[
            "fpga",
            "rtl",
            "verilog",
            "vhdl",
            "asic",
            "synthesis",
            "timing closure",
            "physical design",
            "place and route",
            "floorplan",
            "signal integrity",
            "circuit design",
            "analog design",
            "mixed signal",
            "pcb",
            "hardware design",
        ],
    ),
    (
        "operating systems",
        &[
            "operating system",
            "kernel",
            "linux kernel",
            "scheduler",
            "hypervisor",
            "virtualization",
            "syscall",
            "file system",
            "concurrency",
            "ipc",
            "inter process communication",
            "memory management",
            "driver model",
        ],
    ),
    (
        "compilers",
        &[
            "compiler",
            "llvm",
            "mlir",
            "code generation",
            "optimization pass",
            "jit",
            "instruction selection",
            "register allocation",
            "toolchain",
        ],
    ),
    (
        "thesis",
        &[
            "masterarbeit",
            "abschlussarbeit",
            "bachelorarbeit",
            "diplomarbeit",
            "studienarbeit",
            "master thesis",
            "masters thesis",
            "master s thesis",
            "bachelor thesis",
            "thesis",
            "forschungsarbeit",
            "open topic",
            "offene arbeit",
        ],
    ),
    (
        "research",
        &[
            "research assistant",
            "wissenschaftliche hilfskraft",
            "wissenschaftlicher mitarbeiter",
            "wissenschaftliche mitarbeiterin",
            "hiwi",
            "doktorand",
            "phd",
            "postdoc",
            "research scientist",
            "research engineer",
            "publication",
            "peer reviewed",
        ],
    ),
    (
        "machine learning systems",
        &[
            "machine learning",
            "deep learning",
            "neural network",
            "inference engine",
            "training cluster",
            "gpu",
            "cuda",
            "tensor",
            "quantization",
            "model serving",
        ],
    ),
];

/// The concept a term belongs to, if this vocabulary knows it. A term that matches no concept is still matched literally; it simply gains no expansion.
pub fn concept_for(normalized_term: &str) -> Option<&'static str> {
    CONCEPTS.iter().find_map(|(concept, phrases)| {
        (*concept == normalized_term || phrases.contains(&normalized_term)).then_some(*concept)
    })
}

/// Every surface form of a concept, including the concept name itself.
pub fn phrases_of(concept: &str) -> &'static [&'static str] {
    CONCEPTS
        .iter()
        .find_map(|(name, phrases)| (*name == concept).then_some(*phrases))
        .unwrap_or(&[])
}

/// All known concept names, for the profile editor to offer.
pub fn all_concepts() -> Vec<&'static str> {
    CONCEPTS.iter().map(|(name, _)| *name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_term_resolves_to_its_concept_and_back_to_its_siblings() {
        assert_eq!(concept_for("dram"), Some("memory systems"));
        assert_eq!(concept_for("cxl"), Some("memory systems"));
        assert_eq!(concept_for("rtkit"), Some("firmware"));
        assert_eq!(concept_for("masterarbeit"), Some("thesis"));
        assert_eq!(concept_for("fpga"), Some("hardware design"));
        assert!(phrases_of("memory systems").contains(&"page migration"));
        assert!(phrases_of("firmware").contains(&"secure boot"));
    }

    #[test]
    fn a_concept_name_resolves_to_itself() {
        assert_eq!(concept_for("memory systems"), Some("memory systems"));
        assert_eq!(concept_for("hardware security"), Some("hardware security"));
    }

    #[test]
    fn an_unknown_term_has_no_concept_and_is_left_to_literal_matching() {
        assert_eq!(concept_for("underwater basket weaving"), None);
        assert_eq!(concept_for(""), None);
        assert!(phrases_of("not a concept").is_empty());
    }

    #[test]
    fn every_concept_is_reachable_from_its_own_name() {
        for name in all_concepts() {
            assert_eq!(
                concept_for(name),
                Some(name),
                "{name} did not resolve to itself"
            );
            assert!(!phrases_of(name).is_empty(), "{name} has no surface forms");
        }
    }

    #[test]
    fn concept_phrases_are_stored_already_normalized() {
        for name in all_concepts() {
            for phrase in phrases_of(name) {
                let normalized = crate::normalize::normalize_location(Some(phrase));
                assert_eq!(
                    &normalized, phrase,
                    "phrase '{phrase}' in '{name}' is not in normalized form"
                );
            }
        }
    }
}
