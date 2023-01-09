use super::InstallableKind;
use crate::utils::InstallableDef;

#[derive(Default, Debug, Clone)]
pub struct TemplateInstallable;
impl InstallableDef for TemplateInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] = &[InstallableKind::template()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = Some(&["description"]);
    const SUBCOMMAND: &'static str = "init";
}

#[derive(Default, Debug, Clone)]
pub struct BuildInstallable;
impl InstallableDef for BuildInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] = &[InstallableKind::package()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "build";
}

#[derive(Default, Debug, Clone)]
pub struct DevelopInstallable;
impl InstallableDef for DevelopInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] =
        &[InstallableKind::package(), InstallableKind::shell()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "develop";
}

#[derive(Default, Debug, Clone)]
pub struct PublishInstallable;

impl InstallableDef for PublishInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] = &[InstallableKind::package()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "publish";
}

#[derive(Default, Debug, Clone)]
pub struct RunInstallable;

impl InstallableDef for RunInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] =
        &[InstallableKind::package(), InstallableKind::app()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "run";
}

#[derive(Default, Debug, Clone)]
pub struct ShellInstallable;

impl InstallableDef for ShellInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] = &[InstallableKind::package()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "shell";
}

#[derive(Default, Debug, Clone)]
pub struct BundleInstallable;

impl InstallableDef for BundleInstallable {
    const DERIVATION_TYPES: &'static [InstallableKind] = &[InstallableKind::package()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "bundle";
}

#[derive(Default, Debug, Clone)]
pub struct BundlerInstallable;

impl InstallableDef for BundlerInstallable {
    const ARG_FLAG: Option<&'static str> = Some("--bundler");
    const DERIVATION_TYPES: &'static [InstallableKind] = &[InstallableKind::bundler()];
    const DESCRIPTION_KEY: Option<&'static [&'static str]> = None;
    const SUBCOMMAND: &'static str = "bundle";
}
