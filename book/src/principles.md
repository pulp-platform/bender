# Principles

Bender was created to solve the challenges of managing large-scale hardware designs with complex, nested dependencies. It is built around four core principles that guide its development and usage.

## 1. Modular and Opt-in
Bender is designed to be a "pre-build" tool. It does not replace your EDA tools (synthesis, simulation, formal); it orchestrates them. 
- **Tool Agnostic:** Whether you use Vivado, Questa, VCS, or Verilator, Bender provides the necessary file lists and configurations.
- **Flexible Layout:** We do not enforce a strict directory structure. As long as a `Bender.yml` is present, Bender can manage it.

## 2. Reproducibility as Ground Truth
In hardware design, knowing exactly what was taped out or simulated is critical.
- **Precise Locking:** The `Bender.lock` file tracks every dependency down to its specific Git commit hash.
- **Immutable States:** By committing the lockfile, you ensure that everyone on the team—and every CI runner—is using identical source code.

## 3. Decentralized and Secure
Unlike many software package managers (like npm or cargo), Bender does not rely on a central, public registry.
- **Git-Centric:** Dependencies are resolved directly from Git repositories.
- **NDA Friendly:** This allows projects to use internal, private repositories or even local paths, ensuring sensitive IP remains protected and within your infrastructure.

## 4. Local-First Development
Hardware development often requires modifying an IP and its dependencies simultaneously.
- **Zero-Friction Overrides:** The `Bender.local` mechanism allows you to temporarily swap a remote dependency for a local working copy without changing the project's official manifest.
- **Seamless Snapshots:** Captured states can be shared or moved to CI easily, bridging the gap between local development and official releases.

---

## The Three Tiers of Bender

Bender's functionality can be categorized into three distinct tiers, each building upon the other:

### Tier 1: Source Collection
At its simplest level, Bender is a tool for collecting and organizing HDL source files.
- **Ordering:** Maintains the required order across source files (e.g., packages before modules).
- **Organization:** Allows files to be organized into recursive groups with specific targets, defines, and include directories.

### Tier 2: Dependency Management
Bender resolves and manages transitive dependencies between different hardware IPs.
- **Version Resolution:** Enforces Semantic Versioning (SemVer) to ensure compatibility.
- **Lifecycle Management:** Automates the fetching, checking out, and updating of external packages.

### Tier 3: Tool Script Generation
The final tier provides the ability to generate the actual scripts and file lists used by vendor tools.
- **Automation:** Eliminates the need to manually maintain tool-specific file lists (like `.f` files or TCL scripts).
- **Consistency:** Ensures that the exact same set of sources is used across all stages of the design flow.
