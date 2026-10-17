#!/bin/sh
# pam_exec expose_authtok testi: şifre stdin'den gelir (sonda NUL olabilir). PAM çocuğa PATH
# vermez; mutlak yollar. Sabit test şifresi — gerçek bir kullanıcı şifresi DEĞİL.
pw=$(/usr/bin/tr -d '\000')
[ "$pw" = "nuxut-test-sifresi" ]
