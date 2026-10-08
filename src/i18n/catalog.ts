/**
 * The shape a translated catalog must have: every English key, nothing more.
 * Chinese has one plural form, so an English `_one`/`_other` pair becomes a
 * lone `_other`; that is the only key a translation may leave out.
 * Placeholders are compared by a test (`i18n-catalog`), not by this type.
 */
export type TranslatedCatalog<T> = {
  [K in keyof T as K extends `${infer Base}_one`
    ? `${Base}_other` extends keyof T ? never : K
    : K]: T[K] extends string ? string : TranslatedCatalog<T[K]>;
};
