import { same, type Choice } from "../settingsModel.js";
import type { ControlProps } from "./types.js";

/** A select over the setting's choices (`enum` values, or titled `const`
 *  branches), with the chosen one's description beneath. */
export function EnumField({ info, value, onChange, label, choices }: ControlProps & { choices: Choice[] }) {
  void info;
  const chosen = choices.findIndex((choice) => same(choice.value, value));
  const description = choices[chosen]?.description;
  return (
    <label>
      {label}
      <select
        className="ox-input"
        aria-label={label}
        value={chosen}
        onChange={(e) => onChange(choices[Number(e.target.value)]!.value)}
      >
        <option value={-1} disabled>
          Choose a value
        </option>
        {choices.map((choice, i) => (
          <option key={i} value={i}>
            {choice.title}
          </option>
        ))}
      </select>
      {description && <span className="ox-muted">{description}</span>}
    </label>
  );
}
