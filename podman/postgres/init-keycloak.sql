-- Runs once, on first start of an empty data volume.
CREATE ROLE keycloak LOGIN PASSWORD 'keycloak_dev_password';
CREATE DATABASE keycloak OWNER keycloak;
