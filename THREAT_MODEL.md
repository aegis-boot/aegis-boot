# Secure Boot Threat Model and Security Boundary Document

## 1. Glossary

- **PK (Platform Key)** — Master key in UEFI secure boot that signs KEK
- **KEK (Key Exchange Key)** - Signs DB and DBX key databases
- **MOK (Machine Owner Key)** — User-managed key for unsigned kernel modules
- **db (signature database)** — Allowlist of trusted executables
- **dbx (forbidden signatures database)** — Blocklist of known-bad signatures
- **shim** — First-stage bootloader that delegates to systemd-boot
- **ESP (EFI System Partition)** — FAT32 partition containing bootloaders
- **UEFI (Unified Extensible Firmware Interface)** — Firmware interface standard
- **TOCTOU (Time-of-Check-Time-of-Use)** — Race condition between verification and execution
- **SBAT (Secure Boot Advanced Targeting)** — Revocation mechanism for boot components

---

## 2. Asset Inventory

| Asset ID | Asset Name                         | Type          | Criticality | Owner    |
| -------- | ---------------------------------- | ------------- | ----------- | -------- |
| A1       | Shim bootloader (shimx64.efi)      | Software      | Critical    | Vendor   |
| A2       | systemd-boot (systemd-bootx64.efi) | Software      | Critical    | Project  |
| A3       | Kernel (vmlinuz)                   | Software      | Critical    | Project  |
| A4       | initramfs                          | Data/Software | High        | Project  |
| A5       | MOK keys (PK/KEK/db/dbx)           | Cryptographic | Critical    | Platform |
| A6       | ESP partition                      | Storage       | High        | Platform |
| A7       | UEFI firmware                      | Firmware      | Critical    | Vendor   |
| A8       | Aegis-Boot orchestrator binary     | Software      | High        | Project  |
| A9       | Kernel command line (cmdline.txt)  | Configuration | High        | Project  |
| A10      | Boot entries (EFI variable)        | Configuration | High        | Platform |

---

## 3. Threat Actors

| ID  | Actor                          | Capability                          | Intent     | Opportunity |
| --- | ------------------------------ | ----------------------------------- | ---------- | ----------- |
| TA1 | Opportunistic local attacker   | Physical access, basic technical    | Varies     | High        |
| TA2 | Script kiddie                  | Pre-built exploits                  | High       | Medium      |
| TA3 | Insider (non-privileged)       | Logical access, physical            | Low-medium | Medium      |
| TA4 | Organized crime                | Resources, custom malware           | High       | Low         |
| TA5 | Nation-state APT               | Firmware-level access, supply chain | Very high  | Low         |
| TA6 | Malicious insider (privileged) | Full system access                  | High       | High        |

---

## 4. Trust Boundaries and Data Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        SECURE BOOT CHAIN OF TRUST                          │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────┐    ┌─────────────┐    ┌──────────┐    ┌─────────────────┐    │
│  │  UEFI   │───▶│    shim      │───▶│systemd-  │───▶│     kernel      │    │
│  │ Firmware│    │  (A1)        │    │boot (A2) │    │     (A3)        │    │
│  └─────────┘    └──────┬──────┘    └────┬─────┘    └────────┬────────┘    │
│       │                │                 │                  │             │
│       ▼                ▼                 ▼                  ▼             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                    SECURITY BOUNDARIES                             │   │
│  │  B1: Hardware/Firmware    ── Firmware validates shim signature     │   │
│  │  B2: Shim/Bootloader     ── shim validates systemd-boot            │   │
│  │  B3: Bootloader/Kernel   ── systemd-boot validates kernel          │   │
│  │  B4: Kernel/Initramfs    ── Kernel validates initramfs integrity   │   │
│  │  B5: Kernel/Orchestrator ── Kernel validates orchestrator binary  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Data Flow Summary:**

1. UEFI firmware validates shim (B1)
2. shim loads systemd-boot from ESP, verifies SBAT/DB (B2)
3. systemd-boot reads boot entries, loads kernel + initramfs (B3)
4. Kernel decompresses initramfs, mounts root (B4)
5. systemd launches orchestrator binary (B5)

---

## 5. Chain-of-Trust Assumptions

### A1: UEFI Secure Boot is Enforced at Boot Time

- **Justification:** Platform defaults to enforcing secure boot; users cannot bypass without physically rebooting and entering firmware setup
- **Verification:** `bootctl status` shows `secureBoot:=enforced`

### A2: Shim is Signed by Canonical Key

- **Justification:** shim is signed by Microsoft Corporation UEFI CA 2023; SHA256 hash verified against Microsoft SBAT table
- **Verification:** `sbattest` or check SBAT version against latest revocation list

### A3: systemd-boot is Signed by shim-allowlisted Key

- **Justification:** systemd-boot binary signed by systemd's DB key, included in shim's signature database
- **Verification:** `sbattach --stat <boot-image>` shows valid signature chain

### A4: Kernel is Signed by DB Key or Loading is Controlled

- **Justification:** Kernel signed by Canonical's DB key; unsigned kernels rejected unless secure boot disabled
- **Verification:** `journalctl -b | grep -i "signature.*failed"` should return empty

### A5: MOK Keys are Protected Against Tampering

- **Justification:** MOK enrollment requires physical presence (mokutil --import); keys stored in NVRAM with attribute 'boot-service-only'
- **Verification:** `mokutil --list-enrolled` shows enrolled keys; verify key ownership

---

## 6. ISO Signature Verification Requirements

### 6.1 Signing Infrastructure Requirements

- **Yubikey/FIDO2 hardware token** for key storage (recommended) or dedicated air-gapped signing host
- **HSM (Hardware Security Module)** for production keys — threshold 2-of-3 signing
- **Offline master key** stored in safe, activated only for signing ceremonies

### 6.2 Verification Protocol

```bash
# Step 1: Verify shim signature against Microsoft SBAT
sbattest

# Step 2: Check systemd-boot signature
sbattach --list-sigs /boot/EFI/systemd/systemd-bootx64.efi

# Step 3: Verify kernel image signature
sbsign --verify /boot/vmlinuz-$(uname -r)

# Step 4: Verify initramfs integrity (if signed)
cat /boot/initrd.img-$(uname -r) | sha256sum

# Step 5: Verify orchestrator binary before execution
chattr +i /usr/local/bin/aegis-boot  # immutable attribute
```

### 6.3 Failure Conditions

| Condition                     | Action                        | Escalation |
| ----------------------------- | ----------------------------- | ---------- |
| SBAT version outdated         | Reject boot, notify admin     | P1         |
| Signature verification failed | Halt boot, fallback to rescue | P1         |
| Key revocation detected       | Immediate boot halt           | P1         |
| Immutable bit removed         | Alert SOC, quarantine         | P2         |

### 6.4 Automation Requirements

- Daily SBAT check via cron job
- Automated signature verification on package updates
- Integrity measurement logging to remote audit server
- Alert on any verification failure

---

## 7. Attack Surface Enumeration

### 7.1 Physical Attack Vectors

| ID    | Vector               | Description                           |
| ----- | -------------------- | ------------------------------------- |
| AV-P1 | USB device insertion | BadUSB attack executing at boot       |
| AV-P2 | Cold boot attack     | Memory dump after power cut           |
| AV-P3 | Evil maid attack     | Physical modification during absence  |
| AV-P4 | JTAG debug port      | Direct firmware access                |
| AV-P5 | DMA via Thunderbolt  | Firewire/Thunderbolt DMA extraction   |
| AV-P6 | Boot media removal   | Swap boot drive to compromised system |

### 7.2 Logical Attack Vectors

| ID    | Vector                        | Description                               |
| ----- | ----------------------------- | ----------------------------------------- |
| AV-L1 | Kernel command line injection | Modify cmdline.txt via ESP mount          |
| AV-L2 | initramfs modification        | Inject payload before pivot_root          |
| AV-L3 | Boot entry manipulation       | Modify EFI variable boot order            |
| AV-L4 | MOK key injection             | Enroll malicious key via/shim             |
| AV-L5 | TOCTOU race condition         | Exploit window between verify and execute |
| AV-L6 | GRUB password bypass          | Bypass password via live USB              |

### 7.3 Firmware/Supply Chain Attack Vectors

| ID    | Vector                  | Description                       |
| ----- | ----------------------- | --------------------------------- |
| AV-F1 | Firmware implant        | Pre-installed firmware malware    |
| AV-F2 | Shim downgrade          | Revert to vulnerable shim version |
| AV-F3 | SBAT evasion            | Future SBAT bypass technique      |
| AV-F4 | Supply chain compromise | Compromise during manufacturing   |

---

## 8. STRIDE Analysis

| ID  | Category               | Threat                              | Affected Asset | Severity |
| --- | ---------------------- | ----------------------------------- | -------------- | -------- |
| S1  | Spoofing               | Bootloader impersonation            | A2             | High     |
| S2  | Spoofing               | Fake kernel image injection         | A3             | Critical |
| T1  | Tampering              | Modify initramfs before boot        | A4             | High     |
| T2  | Tampering              | Alter kernel command line           | A9             | Medium   |
| T3  | Tampering              | Modify boot entry order             | A10            | High     |
| T4  | Tampering              | Replace shim with malicious version | A1             | Critical |
| R1  | Repudiation            | No audit trail of boot changes      | All            | Medium   |
| R2  | Repudiation            | Attestation fails to log            | All            | Medium   |
| I1  | Information Disclosure | Cold boot memory dump               | A8             | High     |
| I2  | Information Disclosure | EFI variable exposure               | A10            | Low      |
| D1  | Denial of Service      | Brick UEFI via bad update           | A7             | Critical |
| D2  | Denial of Service      | Corrupt ESP rendering unbootable    | A6             | High     |
| D3  | Denial of Service      | Remove boot entries via variable    | A10            | Medium   |
| E1  | Elevation of Privilege | MOK enrollment for kernel modules   | A5             | Critical |
| E2  | Elevation of Privilege | Bypass secure boot via exploit      | A1             | Critical |

---

## 9. Attack Vectors with DREAD Scoring

### AV-1: USB Thunderbolt Device Injection

- **Attack:** Insert malicious USB-C/Thunderbolt device that executes DMA attack during POST
- **DREAD:** Damage (3) + Reproducibility (2) + Exploitability (2) + Affected Users (3) + Discoverability (2) = **12/20 (Medium)**
- **Mitigation:** Disable Thunderbolt in firmware, use USBGuard

### AV-2: TOCTOU Boot Race

- **Attack:** Exploit window between signature verification and kernel execution to inject payload
- **DREAD:** Damage (4) + Reproducibility (1) + Exploitability (2) + Affected Users (3) + Discoverability (1) = **11/20 (Medium)**
- **Mitigation:** Kernel lockdown mode, measured boot, immutable verification

### AV-3: Boot Entry Injection via EFI Variable

- **Attack:** Modify EFI variable to point to malicious .efi on ESP
- **DREAD:** Damage (4) + Reproducibility (3) + Exploitability (3) + Affected Users (2) + Discoverability (2) = **14/20 (High)**
- **Mitigation:** Firmware password protection, boot guard

### AV-4: MOK Key Enrollment via OOB

- **Attack:** Physical access to trigger MOK enrollment (within 5 second window or via reboot)
- **DREAD:** Damage (5) + Reproducibility (3) + Exploitability (3) + Affected Users (3) + Discoverability (2) = **16/20 (Critical)**
- **Mitigation:** Disable MOK enrollment, require signed MOK requests only

### AV-5: Supply Chain Shim Replacement

- **Attack:** Compromise during OS install or update, replace shim with backdoored version
- **DREAD:** Damage (5) + Reproducibility (2) + Exploitability (1) + Affected Users (5) + Discoverability (1) = **14/20 (High)**
- **Mitigation:** Verify ISO hash, only use official distribution channels

---

## 10. Risk Matrix

| Priority | Risk                            | Score | Action                                   |
| -------- | ------------------------------- | ----- | ---------------------------------------- |
| P1       | MOK key injection via OOB       | 16/20 | Disable MOK enrollment, HSM for keys     |
| P1       | Shim replacement (supply chain) | 14/20 | Verify ISO, SBAT auto-update             |
| P1       | Boot entry injection            | 14/20 | Firmware password, boot guard            |
| P2       | initramfs modification          | 12/20 | Signed initramfs, immutable verification |
| P2       | USB DMA injection               | 12/20 | Disable Thunderbolt, USBGuard            |
| P3       | TOCTOU race                     | 11/20 | Kernel lockdown, measured boot           |
| P3       | Cold boot attack                | 11/20 | Encrypted memory (TPM), memory wipe      |

---

## 11. Incident Response

### Detection Indicators

```bash
# Unusual boot entry added
efibootmgr -v | grep -i "new"

# MOK key enrolled unexpectedly
mokutil --list-enrolled

# Shim signature changed
sbattest

# Secure boot state changed
bootctl status | grep -i secure

# ESP mount point accessed unexpectedly
auditctl -w /boot/efi -p wa -k esp_mod
```

### Response Procedures

1. **Identify**: Run `bootctl status` and `sbattest` to determine state
2. **Contain**: Boot to trusted recovery environment, mount ESP read-only
3. **Eradicate**: Restore known-good shim/systemd-boot from trusted backup
4. **Recover**: Re-enroll only verified keys, regenerate boot entries
5. **Post-incident**: Audit logs, review physical security, rotate keys

### Contacts

- Security Team: security@aegis-boot.example
- On-call: +1-555-0100
- Hardware Vendor: [Vendor support contact]

---

## 12. Review Schedule

| Review Type             | Frequency | Owner         | Last Review |
| ----------------------- | --------- | ------------- | ----------- |
| Full threat model       | Annual    | Security Lead | [DATE]      |
| Attack surface update   | Quarterly | Security Lead | [DATE]      |
| STRIDE analysis refresh | Annual    | Security Lead | [DATE]      |
| Key rotation            | Annual    | Platform Team | [DATE]      |
| SBAT compliance check   | Monthly   | Platform Team | [DATE]      |

---

**Document Version:** 1.0  
**Classification:** Internal  
**Last Updated:** [DATE]  
**Next Review:** [DATE + 1 year]
