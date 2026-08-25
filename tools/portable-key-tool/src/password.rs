use zeroize::Zeroizing;

const MIN_PASSWORD_CHARS: usize = 12;
const MAX_PASSWORD_BYTES: usize = 1024;

pub fn prompt_existing_password() -> Result<Zeroizing<String>, String> {
    let password = Zeroizing::new(
        rpassword::prompt_password("USB key password: ")
            .map_err(|_| "Could not read the key password from the console.".to_string())?,
    );
    validate_password(&password)?;
    Ok(password)
}

pub fn prompt_new_password() -> Result<Zeroizing<String>, String> {
    let password = Zeroizing::new(
        rpassword::prompt_password("Create USB key password: ")
            .map_err(|_| "Could not read the new key password from the console.".to_string())?,
    );
    validate_password(&password)?;
    let confirmation = Zeroizing::new(
        rpassword::prompt_password("Confirm USB key password: ").map_err(|_| {
            "Could not read the password confirmation from the console.".to_string()
        })?,
    );
    if password.as_bytes() != confirmation.as_bytes() {
        return Err("The password confirmation did not match.".into());
    }
    Ok(password)
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!(
            "The USB key password must contain at least {MIN_PASSWORD_CHARS} characters."
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("The USB key password is too long.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_policy_has_clear_bounds() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("correct horse battery staple").is_ok());
        assert!(validate_password(&"x".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
    }
}
